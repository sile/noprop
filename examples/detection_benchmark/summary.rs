//! Summary aggregation: reads raw-result JSON lines and produces a
//! per (variant x workload x mutant) bucket summary. Regenerated from
//! raw results, never accumulated incrementally.

use std::collections::BTreeMap;
use std::io::BufRead;

use crate::raw::Status;

/// Iterations-to-detection bucket boundaries: 1-9, 10-99, 100-999,
/// 1000+.
const DETECTION_BUCKETS: &[usize] = &[10, 100, 1000];

fn bucket_of(detected_at: usize) -> usize {
    DETECTION_BUCKETS
        .iter()
        .position(|bound| detected_at < *bound)
        .unwrap_or(DETECTION_BUCKETS.len())
}

/// Per-task summary across seeds.
#[derive(Debug, Default)]
pub(crate) struct TaskSummary {
    pub trials: usize,
    pub found: usize,
    pub not_found: usize,
    pub gave_up: usize,
    pub aborted: usize,
    /// Sorted iterations-to-detection across `found` trials.
    pub detection_times: Vec<usize>,
    /// Detection-time bucket counts (index = `bucket_of`).
    pub detection_buckets: [usize; 4],
}

impl TaskSummary {
    pub fn detection_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.found as f64 / self.trials as f64
        }
    }

    pub fn median_detection(&self) -> Option<usize> {
        let n = self.detection_times.len();
        if n == 0 {
            return None;
        }
        let mut times = self.detection_times.clone();
        times.sort_unstable();
        Some(times[n / 2])
    }

    /// 25th / 75th percentiles of iterations-to-detection.
    pub fn quartiles(&self) -> Option<(usize, usize)> {
        let n = self.detection_times.len();
        if n == 0 {
            return None;
        }
        let mut times = self.detection_times.clone();
        times.sort_unstable();
        Some((times[n / 4], times[3 * n / 4]))
    }
}

/// Aggregate raw results keyed by (variant, workload, mutant).
pub(crate) type Summaries = BTreeMap<(String, String, String), TaskSummary>;

/// Accumulate one parsed raw result line.
pub(crate) fn accumulate(summaries: &mut Summaries, raw: &ParsedRaw) {
    let entry = summaries
        .entry((
            raw.variant.clone(),
            raw.workload.clone(),
            raw.mutant.clone(),
        ))
        .or_default();
    entry.trials += 1;
    match raw.status {
        Status::Found => {
            entry.found += 1;
            let detected_at = raw.detected_at.unwrap_or(0);
            entry.detection_times.push(detected_at);
            entry.detection_buckets[bucket_of(detected_at)] += 1;
        }
        Status::NotFound => entry.not_found += 1,
        Status::GaveUp => entry.gave_up += 1,
        Status::Aborted => entry.aborted += 1,
    }
}

/// A raw result parsed back from a JSON line. Only the fields the
/// summary needs are extracted; unknown fields are ignored so future
/// format versions stay readable.
pub(crate) struct ParsedRaw {
    pub variant: String,
    pub workload: String,
    pub mutant: String,
    pub status: Status,
    pub detected_at: Option<usize>,
}

/// Parse one raw-result JSON line. Returns `None` for blank lines;
/// returns an error message for malformed lines.
pub(crate) fn parse_line(line: &str) -> Result<Option<ParsedRaw>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let raw = nojson::RawJson::parse(line)
        .map_err(|e: nojson::JsonParseError| format!("invalid raw result line: {e}"))?;
    let value = raw.value();

    let get_str = |key: &str| -> Result<String, String> {
        let member = value
            .to_member(key)
            .map_err(|e: nojson::JsonParseError| format!("field {key:?}: {e}"))?;
        let Some(v) = member.optional() else {
            return Err(format!("missing string field {key:?}"));
        };
        v.try_into()
            .map_err(|e: nojson::JsonParseError| format!("field {key:?} is not a string: {e}"))
    };

    let status = match get_str("status")?.as_str() {
        "found" => Status::Found,
        "not_found" => Status::NotFound,
        "gave_up" => Status::GaveUp,
        "aborted" => Status::Aborted,
        other => return Err(format!("unknown status {other:?}")),
    };
    let detected_at: Option<usize> = match value.to_member("detected_at") {
        Ok(member) => {
            let Some(v) = member.optional() else {
                return Err("detected_at is present but null".to_string());
            };
            if v.kind() == nojson::JsonValueKind::Null {
                None
            } else {
                Some(v.try_into().map_err(|e: nojson::JsonParseError| {
                    format!("detected_at is not a number: {e}")
                })?)
            }
        }
        Err(_) => None,
    };
    Ok(Some(ParsedRaw {
        variant: get_str("variant")?,
        workload: get_str("workload")?,
        mutant: get_str("mutant")?,
        status,
        detected_at,
    }))
}

/// Read raw results from a reader (one JSON line per task) and return
/// the aggregated summaries and the number of malformed lines skipped.
pub(crate) fn read_summaries<R: BufRead>(reader: R) -> (Summaries, usize) {
    let mut summaries = Summaries::new();
    let mut skipped = 0;
    for line in reader.lines() {
        let Ok(line) = line else {
            skipped += 1;
            continue;
        };
        match parse_line(&line) {
            Ok(Some(raw)) => accumulate(&mut summaries, &raw),
            Ok(None) => {}
            Err(_) => skipped += 1,
        }
    }
    (summaries, skipped)
}
