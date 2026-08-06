//! Generator variants: how the input space is sampled and which runner
//! entry point drives the property. The SUT is identical across
//! variants; only the generator and the search policy differ.
//! (`Variant::Base` is the exception: it runs the base SUT as a
//! ground-truth check, not a comparison variant.)
//!
//! The uniform / biased properties report semantic feedback (`event` /
//! `bucket` / `transition`) and a scalar priority (`maximize`) so the
//! same property runs under every search policy; in uniform mode the
//! feedback methods are allocation-free no-ops, so the uniform /
//! biased results match the recorded baseline (the feedback calls draw
//! no random bytes).

use std::time::Instant;

use noprop::{CorpusPolicy, Error, Runner, Stats, TestCaseContext};

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
    /// Targeted search (`Runner::run_targeted`) over the scalar
    /// priority reported by the property.
    Targeted,
    /// Corpus-guided search admitting purely on feature novelty
    /// (`run_corpus_guided_with_policy(SemanticOnly)`).
    SemanticOnly,
    /// Integrated search: semantic corpus plus scalar priority
    /// (`Runner::run_corpus_guided`, i.e. `SemanticWithPriority`).
    SemanticWithPriority,
}

impl Variant {
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "base" => Some(Variant::Base),
            "uniform" => Some(Variant::Uniform),
            "biased" => Some(Variant::Biased),
            "targeted" => Some(Variant::Targeted),
            "semantic-only" => Some(Variant::SemanticOnly),
            "semantic-with-priority" => Some(Variant::SemanticWithPriority),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Variant::Base => "base",
            Variant::Uniform => "uniform",
            Variant::Biased => "biased",
            Variant::Targeted => "targeted",
            Variant::SemanticOnly => "semantic-only",
            Variant::SemanticWithPriority => "semantic-with-priority",
        }
    }
}

/// Comparison variants used by `run-all`.
pub(crate) const VARIANTS: &[Variant] = &[
    Variant::Uniform,
    Variant::Biased,
    Variant::Targeted,
    Variant::SemanticOnly,
    Variant::SemanticWithPriority,
];

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

    let start = Instant::now();
    let mut runner = Runner::new(seed, iterations);
    let outcome = run_variant(&mut runner, task, variant, &observe);
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
        discovered_features: stats.discovered_features,
        max_corpus_size: stats.max_corpus_size,
        observations,
        wall_clock_ns,
    }
}

/// Drive the task's property under the variant's runner entry point.
///
/// The uniform / biased variants use the task's base / biased
/// properties (feedback-reporting under uniform / biased generation);
/// the search variants reuse the uniform property, whose feedback
/// methods are no-ops under `Runner::run`.
fn run_variant(
    runner: &mut Runner,
    task: &Task,
    variant: Variant,
    observe: &Observe,
) -> Result<(), Error> {
    let property: Property = match variant {
        Variant::Base => task.base,
        Variant::Biased => task.biased,
        _ => task.uniform,
    };
    let run = |ctx: &mut TestCaseContext| property(ctx, observe).map_err(Into::into);
    match variant {
        Variant::Base | Variant::Uniform | Variant::Biased => runner.run(run),
        Variant::Targeted => runner.run_targeted(run),
        Variant::SemanticOnly => {
            runner.run_corpus_guided_with_policy(CorpusPolicy::SemanticOnly, run)
        }
        Variant::SemanticWithPriority => runner.run_corpus_guided(run),
    }
}

/// Classify the run outcome: property failure (found), rejection-cap
/// exhaustion (gave_up), harness-level feedback failure (aborted), or
/// a clean pass (not_found).
///
/// `ErrorKind` is crate-private, so the classification relies on the
/// stable `Display` wording of the failure kinds (pinned by the e2e
/// tests). Targeted mode's missing / invalid feedback failures are
/// harness errors of the property, not mutant detections, so they are
/// classified as `Aborted` rather than `Found`.
fn classify(outcome: &Result<(), Error>) -> (Status, Option<usize>) {
    match outcome {
        Ok(()) => (Status::NotFound, None),
        Err(err) => {
            let display = format!("{err}");
            if display.contains("too many rejections") {
                (Status::GaveUp, None)
            } else if display.contains("missing feedback") || display.contains("invalid feedback") {
                (Status::Aborted, None)
            } else {
                // `case_index` is the accepted case that failed; the
                // iterations-to-detection count includes it.
                (Status::Found, Some(err.case_index() + 1))
            }
        }
    }
}
