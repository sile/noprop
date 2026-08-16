//! Generator variants: how the input space is sampled. The SUT is
//! identical across variants; only the generator differs.
//! (`Variant::Base` is the exception: it runs the base SUT as a
//! ground-truth check, not a comparison variant.)

use std::time::Instant;

use crate::raw::{RawResult, Status};
use crate::targets::{Observe, Property, Task};

/// Generator variant for a task.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Variant {
    /// Base SUT under uniform generation. Ground-truth check: must
    /// complete the property for any input. Not part of the
    /// comparison; use via the `run` subcommand.
    Base,
    /// Uniform generation under `noprop::Runner::run`.
    Uniform,
    /// Explicitly biased generation (`match` + weighted choice) under
    /// `noprop::Runner::run`.
    Biased,
    /// Generic type-level boundary mix over the integer primitives
    /// under `noprop::Runner::run`.
    BoundaryBiased,
}

impl Variant {
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "base" => Some(Variant::Base),
            "uniform" => Some(Variant::Uniform),
            "biased" => Some(Variant::Biased),
            "boundary-biased" => Some(Variant::BoundaryBiased),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Variant::Base => "base",
            Variant::Uniform => "uniform",
            Variant::Biased => "biased",
            Variant::BoundaryBiased => "boundary-biased",
        }
    }
}

/// Comparison variants used by `run-all`.
pub(crate) const VARIANTS: &[Variant] =
    &[Variant::Uniform, Variant::Biased, Variant::BoundaryBiased];

/// Run one task (workload x mutant x variant) for a fixed seed and
/// case budget, returning the raw result.
pub(crate) fn run_task(
    workload: &'static str,
    task: &Task,
    variant: Variant,
    seed: u64,
    cases: usize,
) -> RawResult {
    let observe = Observe::default();

    let start = Instant::now();
    let mut runner = noprop::Runner::new(seed);
    let outcome = run_variant(&mut runner, task, variant, &observe, cases);
    let wall_clock_ns = start.elapsed().as_nanos();
    let stats: noprop::Stats = runner.stats();
    let observations = observe.take();

    let (status, detected_at) = classify(&outcome);

    RawResult {
        format_version: crate::raw::FORMAT_VERSION,
        workload,
        mutant: task.mutant,
        variant: variant.as_str(),
        seed,
        cases,
        status,
        detected_at,
        accepted_cases: stats.accepted_cases,
        rejected_cases: stats.rejected_cases,
        total_samples: stats.total_samples,
        observations,
        wall_clock_ns,
    }
}

/// Drive the task's property under the variant's generator.
fn run_variant(
    runner: &mut noprop::Runner,
    task: &Task,
    variant: Variant,
    observe: &Observe,
    cases: usize,
) -> noprop::RunResult {
    let property: Property = match variant {
        Variant::Base => task.base,
        Variant::Uniform => task.uniform,
        Variant::Biased => task.biased,
        Variant::BoundaryBiased => task.bb,
    };
    let run = |ctx: &mut noprop::TestCaseContext| property(ctx, observe).map_err(Into::into);
    runner.run(cases, run)
}

/// Classify the run outcome: property failure (found), rejection-cap
/// exhaustion (gave_up), or a clean pass (not_found).
fn classify(outcome: &noprop::RunResult) -> (Status, Option<usize>) {
    match outcome {
        Ok(()) => (Status::NotFound, None),
        Err(err) => match err.kind() {
            noprop::RunErrorKind::TooManyRejections => (Status::GaveUp, None),
            noprop::RunErrorKind::PropertyFailure => {
                // `case_index` is the accepted case that failed; the
                // cases-to-detection count includes it.
                (Status::Found, Some(err.case_index() + 1))
            }
        },
    }
}
