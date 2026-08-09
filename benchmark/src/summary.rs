//! Summary aggregation: reads raw-result JSON lines and produces a
//! per (variant x workload x mutant) bucket summary. Regenerated from
//! raw results, never accumulated incrementally.

use std::collections::BTreeMap;
use std::io::BufRead;

use crate::raw::{FORMAT_VERSION, Status};

/// Iterations-to-detection bucket boundaries: 1-9, 10-99, 100-999,
/// 1000+.
pub(crate) const DETECTION_BUCKETS: &[usize] = &[10, 100, 1000];

fn bucket_of(detected_at: usize) -> usize {
    DETECTION_BUCKETS
        .iter()
        .position(|bound| detected_at < *bound)
        .unwrap_or(DETECTION_BUCKETS.len())
}

/// Nearest-rank median of `values` (upper median, i.e. index `n / 2`).
fn median_of(values: &[usize]) -> Option<usize> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[n / 2])
}

/// Nearest-rank quartiles of `values` (indices `n / 4` and `3n / 4`).
fn quartiles_of(values: &[usize]) -> Option<(usize, usize)> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some((sorted[n / 4], sorted[3 * n / 4]))
}

/// Per-task summary across seeds.
#[derive(Debug, Default)]
pub(crate) struct TaskSummary {
    pub trials: usize,
    pub found: usize,
    pub not_found: usize,
    pub gave_up: usize,
    pub aborted: usize,
    /// Iterations-to-detection across `found` trials, in aggregation
    /// order; sorted by `median_detection` / `quartiles` on read.
    pub detection_times: Vec<usize>,
    /// Detection-time bucket counts (index = `bucket_of`).
    pub detection_buckets: [usize; 4],
    /// Candidate executions-to-detection across `found` trials
    /// (accepted + rejected + the failing case). One-based; excludes
    /// the search warm-up cost from the cases metric by counting
    /// every candidate.
    pub candidate_times: Vec<usize>,
    /// Candidate-count bucket counts (index = `bucket_of`).
    pub candidate_buckets: [usize; 4],
    /// Distinct observed features per trial (corpus-guided variants
    /// only; uniform / biased report 0).
    pub discovered_features: Vec<usize>,
    /// Combined corpus size per trial (corpus-guided variants only).
    pub max_corpus_sizes: Vec<usize>,
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
        median_of(&self.detection_times)
    }

    /// 25th / 75th percentiles of cases-to-detection (nearest-rank
    /// with the upper median, i.e. index `p * n` of the sorted values).
    pub fn quartiles(&self) -> Option<(usize, usize)> {
        quartiles_of(&self.detection_times)
    }

    pub fn median_candidates(&self) -> Option<usize> {
        median_of(&self.candidate_times)
    }

    pub fn candidate_quartiles(&self) -> Option<(usize, usize)> {
        quartiles_of(&self.candidate_times)
    }

    pub fn median_discovered_features(&self) -> Option<usize> {
        median_of(&self.discovered_features)
    }

    pub fn median_max_corpus_size(&self) -> Option<usize> {
        median_of(&self.max_corpus_sizes)
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
    entry.discovered_features.push(raw.discovered_features);
    entry.max_corpus_sizes.push(raw.max_corpus_size);
    match raw.status {
        Status::Found => {
            entry.found += 1;
            // parse_line guarantees a numeric value for `found`.
            let detected_at = raw
                .detected_at
                .expect("found status always carries a numeric detected_at");
            entry.detection_times.push(detected_at);
            entry.detection_buckets[bucket_of(detected_at)] += 1;
            // Candidate executions-to-detection: the accepted cases,
            // the rejected cases, and the failing case itself (which
            // is neither accepted nor rejected). The run stops at the
            // failure, so the run-end stats equal the detection-point
            // stats.
            let candidates = raw.accepted_cases + raw.rejected_cases + 1;
            entry.candidate_times.push(candidates);
            entry.candidate_buckets[bucket_of(candidates)] += 1;
        }
        Status::NotFound => entry.not_found += 1,
        Status::GaveUp => entry.gave_up += 1,
        Status::Aborted => entry.aborted += 1,
    }
}

/// A raw result parsed back from a JSON line. Only the fields the
/// summary needs are extracted; unknown fields are ignored so future
/// format versions stay readable.
#[derive(Debug)]
pub(crate) struct ParsedRaw {
    pub variant: String,
    pub workload: String,
    pub mutant: String,
    pub status: Status,
    pub detected_at: Option<usize>,
    pub accepted_cases: usize,
    pub rejected_cases: usize,
    pub discovered_features: usize,
    pub max_corpus_size: usize,
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
    let get_u64 = |key: &str| -> Result<u64, String> {
        let member = value
            .to_member(key)
            .map_err(|e: nojson::JsonParseError| format!("field {key:?}: {e}"))?;
        let Some(v) = member.optional() else {
            return Err(format!("missing numeric field {key:?}"));
        };
        v.try_into()
            .map_err(|e: nojson::JsonParseError| format!("field {key:?} is not a number: {e}"))
    };

    // Reject lines from other format versions instead of silently
    // reinterpreting their fields (see `FORMAT_VERSION`).
    let format_version = get_u64("format_version")?;
    if format_version != FORMAT_VERSION as u64 {
        return Err(format!(
            "unsupported format_version {format_version} (expected {FORMAT_VERSION})"
        ));
    }

    let status = match get_str("status")?.as_str() {
        "found" => Status::Found,
        "not_found" => Status::NotFound,
        "gave_up" => Status::GaveUp,
        "aborted" => Status::Aborted,
        other => return Err(format!("unknown status {other:?}")),
    };
    // `detected_at` is either a number (found) or null (other statuses).
    // A missing field is malformed; a `found` line without a numeric
    // value would otherwise be aggregated as if detected in zero
    // cases.
    let member = value
        .to_member("detected_at")
        .map_err(|e: nojson::JsonParseError| format!("field \"detected_at\": {e}"))?;
    let detected_at: Option<usize> =
        {
            let Some(v) = member.optional() else {
                return Err("missing detected_at field".to_string());
            };
            if v.kind() == nojson::JsonValueKind::Null {
                None
            } else {
                Some(v.try_into().map_err(|e: nojson::JsonParseError| {
                    format!("detected_at is not a number: {e}")
                })?)
            }
        };
    if matches!(status, Status::Found) && detected_at.is_none() {
        return Err("found status requires a numeric detected_at".to_string());
    }
    Ok(Some(ParsedRaw {
        variant: get_str("variant")?,
        workload: get_str("workload")?,
        mutant: get_str("mutant")?,
        status,
        detected_at,
        accepted_cases: get_u64("accepted_cases")? as usize,
        rejected_cases: get_u64("rejected_cases")? as usize,
        discovered_features: get_u64("discovered_features")? as usize,
        max_corpus_size: get_u64("max_corpus_size")? as usize,
    }))
}
/// Read raw results from a reader (one JSON line per task) and return
/// the aggregated summaries and the number of malformed lines skipped.
/// Malformed lines are reported to stderr with their line number and
/// reason, so a corrupted artifact is diagnosable.
pub(crate) fn read_summaries<R: BufRead>(reader: R) -> (Summaries, usize) {
    let mut summaries = Summaries::new();
    let mut skipped = 0;
    for (index, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        match parse_line(&line) {
            Ok(Some(raw)) => accumulate(&mut summaries, &raw),
            Ok(None) => {}
            Err(reason) => {
                skipped += 1;
                eprintln!("line {}: {reason}", index + 1);
            }
        }
    }
    (summaries, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal raw-result line for `parse_line` unit tests.
    fn line(status: &str, detected_at: &str) -> String {
        format!(
            r#"{{"format_version":3,"workload":"w","mutant":"m","variant":"v","status":"{status}","detected_at":{detected_at},"accepted_cases":3,"rejected_cases":2,"discovered_features":1,"max_corpus_size":1}}"#
        )
    }

    #[test]
    fn parse_line_reads_a_found_line() {
        let raw = parse_line(&line("found", "7"))
            .expect("found line must parse")
            .expect("non-blank");
        assert!(matches!(raw.status, Status::Found));
        assert_eq!(raw.detected_at, Some(7));
        assert_eq!(raw.accepted_cases, 3);
        assert_eq!(raw.rejected_cases, 2);
        assert_eq!(raw.discovered_features, 1);
        assert_eq!(raw.max_corpus_size, 1);
    }

    #[test]
    fn parse_line_reads_null_detected_at_as_none() {
        let raw = parse_line(&line("not_found", "null"))
            .expect("not_found with null detected_at must parse")
            .expect("non-blank");
        assert!(matches!(raw.status, Status::NotFound));
        assert_eq!(raw.detected_at, None);
    }

    #[test]
    fn parse_line_rejects_wrong_format_version() {
        let line = line("found", "7").replace("\"format_version\":3", "\"format_version\":99");
        let err = parse_line(&line).expect_err("wrong format version must be rejected");
        assert!(err.contains("unsupported format_version 99"), "{err}");
    }

    #[test]
    fn parse_line_rejects_found_without_numeric_detected_at() {
        for input in [
            line("found", "null"),
            line("found", "null").replacen(r#","detected_at":null"#, "", 1),
        ] {
            let err = parse_line(&input)
                .expect_err("found without a numeric detected_at must be rejected");
            assert!(err.contains("detected_at"), "{err}");
        }
    }

    #[test]
    fn parse_line_rejects_missing_required_fields() {
        let input =
            r#"{"format_version":3,"workload":"w","mutant":"m","status":"found","detected_at":1}"#;
        let err = parse_line(input).expect_err("missing variant must be rejected");
        assert!(err.contains("variant"), "{err}");
    }

    #[test]
    fn parse_line_rejects_unknown_status() {
        let err = parse_line(&line("unknown", "1")).expect_err("unknown status must be rejected");
        assert!(err.contains("unknown status"), "{err}");
    }

    #[test]
    fn parse_line_ignores_blank_lines() {
        assert!(
            parse_line("  ")
                .expect("blank line is not an error")
                .is_none()
        );
    }

    #[test]
    fn bucket_of_matches_the_documented_bounds() {
        assert_eq!(bucket_of(1), 0);
        assert_eq!(bucket_of(9), 0);
        assert_eq!(bucket_of(10), 1);
        assert_eq!(bucket_of(99), 1);
        assert_eq!(bucket_of(100), 2);
        assert_eq!(bucket_of(999), 2);
        assert_eq!(bucket_of(1000), 3);
        assert_eq!(bucket_of(usize::MAX), 3);
    }

    #[test]
    fn accumulate_keeps_not_found_trials_in_the_group() {
        let mut summaries = Summaries::new();
        let raw = ParsedRaw {
            variant: "uniform".to_string(),
            workload: "boundary".to_string(),
            mutant: "fails_on_zero".to_string(),
            status: Status::NotFound,
            detected_at: None,
            accepted_cases: 100,
            rejected_cases: 0,
            discovered_features: 0,
            max_corpus_size: 0,
        };
        accumulate(&mut summaries, &raw);
        let entry = summaries
            .get(&(
                "uniform".to_string(),
                "boundary".to_string(),
                "fails_on_zero".to_string(),
            ))
            .expect("group must exist");
        assert_eq!(entry.trials, 1);
        assert_eq!(entry.not_found, 1);
        assert_eq!(entry.detection_times.len(), 0);
        assert_eq!(entry.candidate_times.len(), 0);
    }

    #[test]
    fn accumulate_counts_candidate_executions_on_found() {
        let mut summaries = Summaries::new();
        let raw = ParsedRaw {
            variant: "corpus-guided".to_string(),
            workload: "w".to_string(),
            mutant: "m".to_string(),
            status: Status::Found,
            detected_at: Some(4),
            accepted_cases: 4,
            rejected_cases: 7,
            discovered_features: 1,
            max_corpus_size: 1,
        };
        accumulate(&mut summaries, &raw);
        let entry = summaries
            .get(&(
                "corpus-guided".to_string(),
                "w".to_string(),
                "m".to_string(),
            ))
            .expect("group must exist");
        assert_eq!(entry.found, 1);
        // accepted + rejected + the failing case itself.
        assert_eq!(entry.candidate_times, vec![12]);
        assert_eq!(entry.candidate_buckets[bucket_of(12)], 1);
        assert_eq!(entry.discovered_features, vec![1]);
    }
}
