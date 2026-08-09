//! Generator variants: how the input space is sampled and which runner
//! entry point drives the property. The SUT is identical across
//! variants; only the generator and the search policy differ.
//! (`Variant::Base` is the exception: it runs the base SUT as a
//! ground-truth check, not a comparison variant.)
//!
//! The uniform / biased properties report semantic feedback (`event` /
//! `bucket` / `transition`) so the same property runs under the
//! corpus-guided search; in uniform mode the feedback methods are
//! allocation-free no-ops, so the uniform / biased results match the
//! recorded baseline (the feedback calls draw no random bytes).

use std::time::Instant;

use noprop::{RunErrorKind, RunResult, Runner, Stats, TestCaseContext};

use crate::raw::{RawResult, Status};
use crate::targets::{Observe, Property, Task};

/// Generator / search variant for a task.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Variant {
    /// Base SUT under uniform generation. Ground-truth check: must
    /// complete the property for any input. Not part of the
    /// comparison; use via the `run` subcommand.
    Base,
    /// Uniform generation under `Runner::run`.
    Uniform,
    /// Explicitly biased generation (`match` + weighted choice) under
    /// `Runner::run`.
    Biased,
    /// Generic type-level boundary mix over the integer primitives
    /// under `Runner::run`.
    BoundaryBiased,
    /// Corpus-guided search admitting purely on feature novelty
    /// (`Runner::run_feedback_guided`).
    CorpusGuided,
}

impl Variant {
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "base" => Some(Variant::Base),
            "uniform" => Some(Variant::Uniform),
            "biased" => Some(Variant::Biased),
            "boundary-biased" => Some(Variant::BoundaryBiased),
            "corpus-guided" => Some(Variant::CorpusGuided),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Variant::Base => "base",
            Variant::Uniform => "uniform",
            Variant::Biased => "biased",
            Variant::BoundaryBiased => "boundary-biased",
            Variant::CorpusGuided => "corpus-guided",
        }
    }
}

/// Comparison variants used by `run-all`.
pub(crate) const VARIANTS: &[Variant] = &[
    Variant::Uniform,
    Variant::Biased,
    Variant::BoundaryBiased,
    Variant::CorpusGuided,
];

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
    let mut runner = Runner::new(seed);
    let outcome = run_variant(&mut runner, task, variant, &observe, cases);
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
        cases,
        status,
        detected_at,
        accepted_cases: stats.accepted_cases,
        rejected_cases: stats.rejected_cases,
        total_samples: stats.total_samples,
        discovered_features: stats.discovered_features,
        max_corpus_size: stats.max_corpus_size,
        observations,
        wall_clock_ns,
    }
}

/// Drive the task's property under the variant's runner entry point.
///
/// The uniform / biased / boundary-biased variants use the task's
/// uniform / biased / bb properties (feedback-reporting under the
/// respective generation); the corpus-guided variant reuses the
/// uniform property, whose feedback methods are no-ops under
/// `Runner::run`.
fn run_variant(
    runner: &mut Runner,
    task: &Task,
    variant: Variant,
    observe: &Observe,
    cases: usize,
) -> RunResult {
    let property: Property = match variant {
        Variant::Base => task.base,
        Variant::Biased => task.biased,
        Variant::BoundaryBiased => task.bb,
        _ => task.uniform,
    };
    let run = |ctx: &mut TestCaseContext| property(ctx, observe).map_err(Into::into);
    match variant {
        Variant::Base | Variant::Uniform | Variant::Biased | Variant::BoundaryBiased => {
            runner.run(cases, run)
        }
        Variant::CorpusGuided => runner.run_feedback_guided(cases, run),
    }
}

/// Classify the run outcome: property failure (found), rejection-cap
/// exhaustion (gave_up), or a clean pass (not_found).
fn classify(outcome: &RunResult) -> (Status, Option<usize>) {
    match outcome {
        Ok(()) => (Status::NotFound, None),
        Err(err) => match err.kind() {
            RunErrorKind::TooManyRejections => (Status::GaveUp, None),
            RunErrorKind::PropertyFailure => {
                // `case_index` is the accepted case that failed; the
                // cases-to-detection count includes it.
                (Status::Found, Some(err.case_index() + 1))
            }
            // The benchmark never declares a required event, so this
            // failure mode cannot occur here.
            RunErrorKind::RequiredEventNotReached => {
                unreachable!("the detection benchmark does not use Runner::require_event")
            }
        },
    }
}
