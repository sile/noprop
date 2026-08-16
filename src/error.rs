//! Error and result types for [`Runner::run`](crate::Runner::run).

use std::panic::Location;

use crate::GeneratedValue;
use crate::runner::Stats;

/// Result alias for `#[test]` functions and property closures.
///
/// `T` defaults to `()`, so a plain `noprop::TestResult` is
/// `Result<(), Box<dyn std::error::Error>>`. Every failure a test can
/// hit — a [`RunError`] from the runner, or a config error from the
/// env helpers — converts into the boxed error via `?`.
pub type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Result alias for [`Runner::run`](crate::Runner::run).
///
/// Unlike [`TestResult`], the error stays type-safe: callers can
/// inspect the seed, case index, generated-value trace, stats, and
/// reproduce hint via [`RunError`]'s accessors and dispatch on the
/// failure kind via [`RunError::kind`].
pub type RunResult = std::result::Result<(), RunError>;

/// Failure information from a [`Runner::run`](crate::Runner::run)
/// invocation.
///
/// A property failure (panic or returned `Err`) is deterministically
/// reproducible: rerunning
/// `noprop::Runner::new(err.seed()).run(N, |ctx| ...)` with the `N`
/// printed by the reproduce hint will hit the same failure again. The
/// hint reuses the original case budget so the rerun also hits the same
/// rejection cap.
///
/// A `TooManyRejections` failure — raised when
/// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case) fires so often that
/// the internal global limit is reached — reports the number of
/// accepted cases that completed before the runner gave up as
/// `case_index()`, so the same seed and case budget reproduce the
/// same exit.
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
/// case budget:
///
/// ```text
/// reproduce with: noprop::Runner::new(0x...).run(N, |ctx| ...)
/// ```
///
/// The closure body is a placeholder: the caller substitutes the
/// original property closure before rerunning, and the re-run size
/// never needs to be recomputed by hand.
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
    stats: Stats,
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
/// future failure axes extend this enum deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunErrorKind {
    /// The property closure returned `Err` or panicked in a case.
    PropertyFailure,
    /// The internal global rejection limit was reached before
    /// `Runner::run`'s `cases` budget of accepted cases completed.
    TooManyRejections,
}

/// The internal, payload-carrying failure mode of a [`RunError`].
enum ErrorKind {
    /// The property closure panicked in this case (typically via
    /// `assert!` / `assert_eq!` or an explicit `panic!`).
    Panic { message: String },
    /// The internal global rejection limit was reached before
    /// `Runner::run`'s `cases` budget of accepted cases completed.
    /// Rejected cases do not count toward the budget, but the runner
    /// keeps track of total attempts (accepted + rejected) via a
    /// crate-private constant so a generator that always rejects still
    /// terminates.
    TooManyRejections {
        rejected_cases: usize,
        last_reject_location: &'static Location<'static>,
    },
}

impl RunErrorKind {
    fn of(kind: &ErrorKind) -> Self {
        match kind {
            ErrorKind::Panic { .. } => RunErrorKind::PropertyFailure,
            ErrorKind::TooManyRejections { .. } => RunErrorKind::TooManyRejections,
        }
    }
}

impl RunError {
    /// Build a property-failure error from the recorded run stats.
    ///
    /// The case index is taken from `stats.accepted_cases`: the caller
    /// must have recorded the progress counters (via `record_stats`)
    /// before constructing the error.
    pub(crate) fn from_panic(
        seed: u64,
        cases: usize,
        message: String,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            stats.accepted_cases,
            cases,
            ErrorKind::Panic { message },
            generated,
            stats,
        )
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
        )
    }

    fn new(
        seed: u64,
        case_index: usize,
        cases: usize,
        kind: ErrorKind,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self {
            seed,
            case_index,
            cases,
            kind,
            generated,
            stats,
        }
    }

    /// The seed that was passed to the [`Runner`](crate::Runner) that
    /// produced this failure.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The count of accepted iterations that completed before the
    /// failure.
    ///
    /// Under [`Runner::run`](crate::Runner::run), a property panic /
    /// returned `Err` fails within the (`case_index + 1`)th accepted
    /// iteration, so this is also the zero-based index of the
    /// failing iteration. For `TooManyRejections`, this is the count
    /// of accepted cases that completed before the runner gave up.
    pub fn case_index(&self) -> usize {
        self.case_index
    }

    /// The generated values recorded during the failing case, in call
    /// order. This is a debugging trace, not a stack backtrace.
    pub fn generated(&self) -> &[GeneratedValue] {
        &self.generated
    }

    /// Observability counters as of the failure. `accepted_cases`
    /// matches [`case_index`](Self::case_index) (accepted cases
    /// completed *before* the failure). `total_samples` includes
    /// samples drawn by the failing case itself.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The failure kind of this error, for type-safe dispatch.
    ///
    /// Prefer this over string-matching the `Display` / `Debug` output
    /// when branching on the failure mode.
    pub fn kind(&self) -> RunErrorKind {
        RunErrorKind::of(&self.kind)
    }
}

impl RunError {
    /// The reproduce command shared by
    /// [`Debug`](std::fmt::Debug) and [`Display`](std::fmt::Display).
    /// Reuses the original case budget so reruns hit the same
    /// rejection cap.
    fn reproduce_command(&self) -> String {
        format!(
            "noprop::Runner::new({:#018x}).run({}, |ctx| ...)",
            self.seed, self.cases
        )
    }
}

impl std::fmt::Debug for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "RunError {{")?;
        writeln!(f, "    seed: {:#018x},", self.seed)?;
        writeln!(f, "    case_index: {},", self.case_index)?;
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
        }
        writeln!(f, "    reproduce: {},", self.reproduce_command())?;
        writeln!(
            f,
            "    stats: {{ accepted: {}, rejected: {}, total_samples: {} }},",
            self.stats.accepted_cases, self.stats.rejected_cases, self.stats.total_samples,
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
        }
        writeln!(f, "reproduce with: {}", self.reproduce_command())?;
        writeln!(
            f,
            "stats: accepted={}, rejected={}, total_samples={}",
            self.stats.accepted_cases, self.stats.rejected_cases, self.stats.total_samples,
        )?;
        if !self.generated.is_empty() {
            writeln!(f, "Generated values:")?;
            for entry in &self.generated {
                writeln!(f, "  {entry:?}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for RunError {}
