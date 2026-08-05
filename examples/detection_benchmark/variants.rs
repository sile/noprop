//! Generator variants: how the input space is sampled. The property
//! and SUT are identical across variants; only the generator differs.
//! (`Variant::Base` is the exception: it runs the base SUT as a
//! ground-truth check, not a comparison variant.)

use std::time::Instant;

use noprop::{Error, Runner, Stats};

use crate::raw::{RawResult, Status};
use crate::targets::{Observe, Task};

/// Generator variant for a task.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Variant {
    /// Base SUT under uniform generation. Ground-truth check: must
    /// complete the property for any input. Not part of the
    /// comparison; use via the `run` subcommand.
    Base,
    /// Uniform generation.
    Uniform,
    /// Explicitly biased generation (`match` + weighted choice).
    Biased,
}

impl Variant {
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "base" => Some(Variant::Base),
            "uniform" => Some(Variant::Uniform),
            "biased" => Some(Variant::Biased),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Variant::Base => "base",
            Variant::Uniform => "uniform",
            Variant::Biased => "biased",
        }
    }
}

/// Comparison variants used by `run-all`.
pub(crate) const VARIANTS: &[Variant] = &[Variant::Uniform, Variant::Biased];

/// Run one task (workload x mutant x variant) for a fixed seed and
/// iteration budget, returning the raw result.
pub(crate) fn run_task(
    workload: &'static str,
    task: &Task,
    variant: Variant,
    seed: u64,
    iterations: usize,
) -> RawResult {
    let observe = Observe::default();
    let property = match variant {
        Variant::Base => task.base,
        Variant::Uniform => task.uniform,
        Variant::Biased => task.biased,
    };

    let start = Instant::now();
    let mut runner = Runner::new(seed, iterations);
    let outcome = runner.run(|ctx| property(ctx, &observe).map_err(Into::into));
    let wall_clock_ns = start.elapsed().as_nanos();
    let stats: Stats = runner.stats();
    let observations = observe.take();

    let (status, detected_at) = classify(&outcome);

    RawResult {
        format_version: crate::raw::FORMAT_VERSION,
        workload,
        mutant: task.mutant,
        variant: variant.as_str(),
        seed,
        iterations,
        status,
        detected_at,
        accepted_iterations: stats.accepted_iterations,
        rejected_iterations: stats.rejected_iterations,
        total_samples: stats.total_samples,
        observations,
        wall_clock_ns,
    }
}

/// Classify the run outcome: property failure (found), rejection-cap
/// exhaustion (gave_up), or a clean pass (not_found).
///
/// `ErrorKind` is crate-private, so the classification relies on the
/// stable `Display` wording of the too-many-rejections failure (pinned
/// by the e2e tests). Non-rejection errors are property failures:
/// `Runner::run` produces no other error kind today.
fn classify(outcome: &Result<(), Error>) -> (Status, Option<usize>) {
    match outcome {
        Ok(()) => (Status::NotFound, None),
        Err(err) => {
            let display = format!("{err}");
            if display.contains("too many rejections") {
                (Status::GaveUp, None)
            } else {
                // `case_index` is the accepted case that failed; the
                // iterations-to-detection count includes it.
                (Status::Found, Some(err.case_index() + 1))
            }
        }
    }
}
