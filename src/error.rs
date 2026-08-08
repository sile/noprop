//! Error and result types for [`Runner::run`](crate::Runner::run) and
//! [`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided).

use std::panic::Location;

use crate::GeneratedValue;
use crate::rng::Feature;
use crate::runner::Stats;

/// Result alias for `#[test]` functions and property closures.
///
/// `T` defaults to `()`, so a plain `noprop::TestResult` is
/// `Result<(), Box<dyn std::error::Error>>`. Every failure a test can
/// hit — a [`RunError`] from the runner, or a config error from the
/// env helpers — converts into the boxed error via `?`.
pub type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Result alias for [`Runner::run`](crate::Runner::run) and
/// [`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided).
///
/// Unlike [`TestResult`], the error stays type-safe: callers can
/// inspect the seed, case index, generated-value trace, stats, and
/// reproduce hint via [`RunError`]'s accessors and dispatch on the
/// failure kind via [`RunError::kind`].
pub type RunResult = std::result::Result<(), RunError>;

/// Failure information from a [`Runner::run`](crate::Runner::run) or
/// [`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided)
/// invocation.
///
/// A property failure (panic or returned `Err`) is deterministically
/// reproducible: rerunning `noprop::Runner::new(err.seed(), N)` with
/// the `N` printed by the reproduce hint will hit the same failure
/// again. The hint reuses the original iteration budget so the rerun
/// also hits the same rejection cap.
///
/// A `TooManyRejections` failure — raised when
/// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case) fires so often that
/// the internal global limit is reached — reports the number of
/// accepted cases that completed before the runner gave up as
/// `case_index()`, so the same seed and iteration budget reproduce the
/// same exit.
///
/// A feedback-guided failure report additionally carries the semantic
/// features the failing case reported and a one-based candidate index.
/// The candidate index counts every attempt — accepted, rejected, and
/// the failing case itself — so it relates to the zero-based
/// `case_index()` (accepted cases only) as
/// `candidate_index = case_index + 1 + rejections before the failure`.
/// For `TooManyRejections`, the candidate index is the ordinal of the
/// last rejected attempt (`accepted + rejected`) and the semantic
/// features are those of that last rejected case.
///
/// `generated()` returns the sequence of values every primitive
/// generator produced during the failing case, together with each call
/// site's source location. For `TooManyRejections`, `generated()`
/// returns the (discarded) trace of the last rejected iteration, since
/// no accepted iteration produced the failure.
///
/// The `Debug` and `Display` output includes the failure message
/// captured from the user's closure along with the generated-value
/// list, so returning this from a `#[test]` function prints a
/// self-contained failure report through the standard test harness.
/// Both formats also print a reproduce hint reusing the original
/// iteration budget:
///
/// ```text
/// reproduce with: noprop::Runner::new(0x..., N)
/// ```
///
/// The hint reuses the original iteration budget and names the failing
/// entry point: `run`'s hint prints the bare constructor, while the
/// feedback-guided hint appends
/// `run_feedback_guided(|ctx| ...)`
/// with the closure body left as a
/// placeholder. In each case the original property closure must be
/// supplied before rerunning; the re-run size never needs to be
/// recomputed by hand.
pub struct RunError {
    seed: u64,
    case_index: usize,
    /// The case budget the failing run was given. The reproduce
    /// hint reuses it so reruns hit the same rejection cap (a
    /// `case_index + 1` hint would shrink the cap and turn the failure
    /// into `TooManyRejections`).
    cases: usize,
    kind: ErrorKind,
    generated: Vec<GeneratedValue>,
    // Boxed to keep `RunError` small: with the corpus stats fields
    // inline, `RunError` is exactly 128 bytes, which already triggers
    // clippy's `result_large_err` (its threshold comparison is
    // `>= 128`). Boxed, `RunError` is 96 bytes — and its size no longer
    // grows when `Stats` gains fields.
    stats: Box<Stats>,
    /// The runner entry point that produced this failure. Switches the
    /// reproduce hint.
    policy: SearchPolicy,
    /// Semantic details of the failing case (corpus-guided runs only;
    /// `None` otherwise).
    semantic: Option<Box<SemanticFailureReport>>,
}

/// Semantic details carried by a corpus-guided failure report.
///
/// Boxed so `RunError` stays small: the fields are only populated on the
/// corpus-guided failure path, and the uniform / targeted entry points
/// never touch them.
struct SemanticFailureReport {
    /// Semantic features the failing case reported.
    features: Vec<Feature>,
    /// The one-based index of the failing candidate across accepted
    /// and rejected cases (the failing case itself included).
    candidate_index: usize,
}

/// The runner entry point that produced an [`RunError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchPolicy {
    /// [`Runner::run`](crate::Runner::run).
    Uniform,
    /// [`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided).
    CorpusGuided,
}

/// The kind of a [`RunError`], for type-safe dispatch on the failure
/// mode.
///
/// Field-less on purpose: the payload (e.g. the rejected-iteration
/// count and last reject location of [`RunErrorKind::TooManyRejections`])
/// stays on [`RunError`] itself, so the failure report keeps its full
/// detail while the kind stays cheap to compare.
///
/// New kinds are added as breaking changes (no `#[non_exhaustive]`), so
/// future failure axes — such as an unmet required-event coverage —
/// extend this enum deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunErrorKind {
    /// The property closure returned `Err` or panicked in a case.
    PropertyFailure,
    /// The internal global rejection limit was reached before
    /// `Runner::run`'s `cases` budget of accepted cases completed.
    TooManyRejections,
    /// The event declared via
    /// [`Runner::require_event`](crate::Runner::require_event) was
    /// never reported during the run, so the required coverage was
    /// not met.
    RequiredEventNotReached,
}

/// The internal, payload-carrying failure mode of a [`RunError`].
enum ErrorKind {
    /// The property closure panicked in this case (typically via
    /// `assert!` / `assert_eq!` or an explicit `panic!`).
    Panic { message: String },
    /// The internal global rejection limit was reached before
    /// `Runner::run`'s `cases` budget of accepted cases completed.
    /// Rejected
    /// cases do not count toward the budget, but the
    /// runner keeps track of total attempts (accepted + rejected) via
    /// a crate-private constant so a generator that always rejects
    /// still terminates.
    TooManyRejections {
        rejected_cases: usize,
        last_reject_location: &'static Location<'static>,
    },
    /// The event declared via
    /// [`Runner::require_event`](crate::Runner::require_event) was
    /// never reported during the run.
    RequiredEventNotReached { label: &'static str },
}

impl RunErrorKind {
    fn of(kind: &ErrorKind) -> Self {
        match kind {
            ErrorKind::Panic { .. } => RunErrorKind::PropertyFailure,
            ErrorKind::TooManyRejections { .. } => RunErrorKind::TooManyRejections,
            ErrorKind::RequiredEventNotReached { .. } => RunErrorKind::RequiredEventNotReached,
        }
    }
}

impl RunError {
    /// Build a property-failure error from the recorded run stats.
    ///
    /// The case index is taken from `stats.accepted_cases`: the caller
    /// must have recorded the progress counters (via `record_stats` /
    /// `record_corpus_stats`) before constructing the error.
    pub(crate) fn from_panic(
        seed: u64,
        cases: usize,
        message: String,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        policy: SearchPolicy,
    ) -> Self {
        Self::new(
            seed,
            stats.accepted_cases,
            cases,
            ErrorKind::Panic { message },
            generated,
            stats,
            policy,
        )
    }

    /// Attach the semantic features and candidate index of a failing
    /// corpus-guided case. Used by
    /// [`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided)
    /// and the too-many-rejections exit path.
    pub(crate) fn with_semantic(mut self, features: Vec<Feature>, candidate_index: usize) -> Self {
        self.semantic = Some(Box::new(SemanticFailureReport {
            features,
            candidate_index,
        }));
        self
    }

    /// Build a too-many-rejections error from the recorded run stats.
    ///
    /// The accepted and rejected case counts are taken from
    /// `stats.accepted_cases` / `stats.rejected_cases`: the caller must
    /// have recorded the progress counters before constructing the
    /// error.
    pub(crate) fn from_too_many_rejections(
        seed: u64,
        cases: usize,
        last_reject_location: &'static Location<'static>,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        policy: SearchPolicy,
    ) -> Self {
        Self::new(
            seed,
            stats.accepted_cases,
            cases,
            ErrorKind::TooManyRejections {
                rejected_cases: stats.rejected_cases,
                last_reject_location,
            },
            generated,
            stats,
            policy,
        )
    }

    /// Build a required-event-not-reached error from the recorded run
    /// stats.
    ///
    /// The run completed its case budget without a single report of
    /// the declared required event, so no failing case exists: the
    /// generated-value trace is empty and `case_index` is the number of
    /// accepted cases that ran.
    pub(crate) fn from_required_event_not_reached(
        seed: u64,
        cases: usize,
        label: &'static str,
        stats: Stats,
        policy: SearchPolicy,
    ) -> Self {
        Self::new(
            seed,
            stats.accepted_cases,
            cases,
            ErrorKind::RequiredEventNotReached { label },
            Vec::new(),
            stats,
            policy,
        )
    }

    fn new(
        seed: u64,
        case_index: usize,
        cases: usize,
        kind: ErrorKind,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        policy: SearchPolicy,
    ) -> Self {
        Self {
            seed,
            case_index,
            cases,
            kind,
            generated,
            stats: Box::new(stats),
            policy,
            semantic: None,
        }
    }

    /// The seed that was passed to the [`Runner`](crate::Runner) that
    /// produced this failure.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The zero-based index of the accepted iteration this failure is
    /// tied to. For a property panic / returned `Err`, this is the
    /// index of the failing iteration. For `TooManyRejections`, this
    /// is the count of accepted cases that ran before the runner
    /// gave up (i.e. the index of the iteration that could not be
    /// accepted).
    pub fn case_index(&self) -> usize {
        self.case_index
    }

    /// The generated values recorded during the failing case, in call
    /// order. This is a debugging trace, not a stack backtrace.
    pub fn generated(&self) -> &[GeneratedValue] {
        &self.generated
    }

    /// Observability counters accumulated up to (and including) the
    /// failing case. `accepted_cases` matches
    /// [`case_index`](Self::case_index). The corpus fields
    /// (`discovered_features` / `max_corpus_size`) do not include the
    /// failing case's features (the failing case is not admitted).
    pub fn stats(&self) -> Stats {
        *self.stats
    }

    /// The failure kind of this error, for type-safe dispatch.
    ///
    /// Prefer this over string-matching the `Display` / `Debug` output
    /// when branching on the failure mode.
    pub fn kind(&self) -> RunErrorKind {
        RunErrorKind::of(&self.kind)
    }

    /// The label of the required event that was never reported, for a
    /// [`RunErrorKind::RequiredEventNotReached`] failure; `None`
    /// otherwise.
    pub fn required_event_label(&self) -> Option<&'static str> {
        match self.kind {
            ErrorKind::RequiredEventNotReached { label } => Some(label),
            _ => None,
        }
    }
}

impl RunError {
    /// The reproduce command shared by
    /// [`Debug`](std::fmt::Debug) and [`Display`](std::fmt::Display).
    /// Reuses the original case budget so reruns hit the same
    /// rejection cap. In corpus-guided mode the closure body
    /// is a placeholder: the caller substitutes the original property
    /// closure.
    fn reproduce_command(&self) -> String {
        match self.policy {
            SearchPolicy::Uniform => format!(
                "noprop::Runner::new({:#018x}).run({}, |ctx| ...)",
                self.seed, self.cases
            ),
            SearchPolicy::CorpusGuided => format!(
                "noprop::Runner::new({:#018x}).run_feedback_guided({}, |ctx| ...)",
                self.seed, self.cases
            ),
        }
    }
}

impl std::fmt::Debug for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "RunError {{")?;
        writeln!(f, "    seed: {:#018x},", self.seed)?;
        writeln!(f, "    case_index: {},", self.case_index)?;
        if let Some(report) = &self.semantic {
            writeln!(f, "    candidate_index: {},", report.candidate_index)?;
        }
        writeln!(f, "    policy: {:?},", self.policy)?;
        match &self.kind {
            ErrorKind::Panic { message } => {
                writeln!(f, "    panic: {message:?},")?;
            }
            ErrorKind::TooManyRejections {
                rejected_cases,
                last_reject_location,
            } => {
                writeln!(
                    f,
                    "    too_many_rejections: {{ rejected: {rejected_cases}, last_reject_at: {}:{} }},",
                    last_reject_location.file(),
                    last_reject_location.line(),
                )?;
            }
            ErrorKind::RequiredEventNotReached { label } => {
                writeln!(f, "    required_event_not_reached: {label:?},")?;
            }
        }
        writeln!(f, "    reproduce: {},", self.reproduce_command())?;
        writeln!(
            f,
            "    stats: {{ accepted: {}, rejected: {}, total_samples: {}, discovered_features: {}, max_corpus_size: {}, required_event_hits: {} }},",
            self.stats.accepted_cases,
            self.stats.rejected_cases,
            self.stats.total_samples,
            self.stats.discovered_features,
            self.stats.max_corpus_size,
            self.stats.required_event_hits,
        )?;
        if self.generated.is_empty() {
            writeln!(f, "    generated: [],")?;
        } else {
            writeln!(f, "    generated: [")?;
            for entry in &self.generated {
                writeln!(f, "        {entry:?}")?;
            }
            writeln!(f, "    ],")?;
        }
        if let Some(report) = &self.semantic
            && !report.features.is_empty()
        {
            writeln!(f, "    semantic_features: [")?;
            for feature in &report.features {
                writeln!(f, "        {},", feature.display_repr())?;
            }
            writeln!(f, "    ],")?;
        }
        write!(f, "}}")
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ErrorKind::Panic { message } => {
                writeln!(
                    f,
                    "noprop failure at case {} (seed={:#018x}): {}",
                    self.case_index, self.seed, message
                )?;
            }
            ErrorKind::TooManyRejections {
                rejected_cases,
                last_reject_location,
            } => {
                writeln!(
                    f,
                    "noprop too many rejections at case {} (seed={:#018x}): \
                     {rejected_cases} rejected case(s), last reject at {}:{}",
                    self.case_index,
                    self.seed,
                    last_reject_location.file(),
                    last_reject_location.line(),
                )?;
            }
            ErrorKind::RequiredEventNotReached { label } => {
                writeln!(
                    f,
                    "noprop required event not reached at case {} (seed={:#018x}): \
                     `{label}` was never reported during the run",
                    self.case_index, self.seed,
                )?;
            }
        }
        writeln!(f, "reproduce with: {}", self.reproduce_command())?;
        writeln!(
            f,
            "stats: accepted={}, rejected={}, total_samples={}, discovered_features={}, max_corpus_size={}, required_event_hits={}",
            self.stats.accepted_cases,
            self.stats.rejected_cases,
            self.stats.total_samples,
            self.stats.discovered_features,
            self.stats.max_corpus_size,
            self.stats.required_event_hits,
        )?;
        if !self.generated.is_empty() {
            writeln!(f, "Generated values:")?;
            for entry in &self.generated {
                writeln!(f, "  {entry:?}")?;
            }
        }
        if let Some(report) = &self.semantic
            && !report.features.is_empty()
        {
            writeln!(f, "Semantic features:")?;
            for feature in &report.features {
                writeln!(f, "  {}", feature.display_repr())?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for RunError {}
