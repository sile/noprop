//! Raw per-seed task result, serialized as one JSON line.

use nojson::DisplayJson;

/// Version of the raw-result format. Bump when the meaning of any field
/// changes so old artifacts are not silently reinterpreted.
pub(crate) const FORMAT_VERSION: u32 = 2;

/// Terminal state of one task trial.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Status {
    /// The mutant was detected within the iteration budget.
    Found,
    /// The run completed without detecting the mutant.
    NotFound,
    /// The run gave up (rejection cap exceeded) before completing.
    GaveUp,
    /// The harness itself failed (configuration error, not a property
    /// failure). No workload generates this today; the status keeps
    /// the summary schema stable for future harness errors.
    Aborted,
}

impl Status {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Status::Found => "found",
            Status::NotFound => "not_found",
            Status::GaveUp => "gave_up",
            Status::Aborted => "aborted",
        }
    }
}

/// One seed's raw result for a single task.
///
/// Serialized with nojson so the format is stable and machine-readable.
#[derive(Debug)]
pub(crate) struct RawResult {
    pub format_version: u32,
    pub workload: &'static str,
    pub mutant: &'static str,
    pub variant: &'static str,
    pub seed: u64,
    pub iterations: usize,
    pub status: Status,
    /// Iterations-to-detection: the zero-based accepted case index of
    /// the failing case plus one (i.e. the number of accepted cases
    /// before the failure, including the failing one). `None` unless
    /// `status` is `Found`.
    pub detected_at: Option<usize>,
    pub accepted_iterations: usize,
    pub rejected_iterations: usize,
    pub total_samples: usize,
    /// Distinct semantic features registered in the global observation
    /// set (corpus-guided variants only; 0 otherwise).
    pub discovered_features: usize,
    /// Combined accepted + rejected corpus size at the end of the run
    /// (corpus-guided variants only; 0 otherwise).
    pub max_corpus_size: usize,
    /// Workload-specific observations gathered during the run
    /// (e.g. the dependent workload's semantic-bucket reach counts).
    pub observations: Vec<(&'static str, u64)>,
    pub wall_clock_ns: u128,
}

impl DisplayJson for RawResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("format_version", self.format_version)?;
            f.member("workload", self.workload)?;
            f.member("mutant", self.mutant)?;
            f.member("variant", self.variant)?;
            f.member("seed", self.seed)?;
            f.member("iterations", self.iterations)?;
            f.member("status", self.status.as_str())?;
            f.member("detected_at", self.detected_at)?;
            f.member("accepted_iterations", self.accepted_iterations)?;
            f.member("rejected_iterations", self.rejected_iterations)?;
            f.member("total_samples", self.total_samples)?;
            f.member("discovered_features", self.discovered_features)?;
            f.member("max_corpus_size", self.max_corpus_size)?;
            f.member(
                "observations",
                nojson::array(|f| {
                    for (label, value) in &self.observations {
                        f.element(nojson::object(|f| {
                            f.member("label", *label)?;
                            f.member("value", *value)
                        }))?;
                    }
                    Ok(())
                }),
            )?;
            f.member("wall_clock_ns", self.wall_clock_ns)
        })
    }
}
