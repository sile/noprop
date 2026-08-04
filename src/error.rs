//! Error and result types for [`Runner::run`](crate::Runner::run) and
//! [`Runner::run_targeted`](crate::Runner::run_targeted).

use std::panic::Location;

use crate::GeneratedValue;
use crate::runner::Stats;

/// Result alias used across noprop's public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure information from a [`Runner::run`](crate::Runner::run) or
/// [`Runner::run_targeted`](crate::Runner::run_targeted) invocation.
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
/// accepted iterations that completed before the runner gave up as
/// `case_index()`, so the same seed and iteration budget reproduce the
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
/// iteration budget:
///
/// ```text
/// reproduce with: noprop::Runner::new(0x..., N)
/// ```
///
/// The hint reuses the original iteration budget and names the failing
/// entry point: `run`'s hint prints the bare constructor, while the
/// targeted hint appends `run_targeted(|ctx| ...)` with the closure
/// body left as a placeholder. In both cases the original property
/// closure must be supplied before rerunning; the re-run size never
/// needs to be recomputed by hand.
pub struct Error {
    seed: u64,
    case_index: usize,
    /// The iteration budget the failing run was given. The reproduce
    /// hint reuses it so reruns hit the same rejection cap (a
    /// `case_index + 1` hint would shrink the cap and turn the failure
    /// into `TooManyRejections`).
    iterations: usize,
    kind: ErrorKind,
    generated: Vec<GeneratedValue>,
    stats: Stats,
    /// `true` when the failure came from
    /// [`Runner::run_targeted`](crate::Runner::run_targeted). Switches
    /// the reproduce hint to the targeted entry point.
    targeted: bool,
}

enum ErrorKind {
    /// The property closure panicked in this case (typically via
    /// `assert!` / `assert_eq!` or an explicit `panic!`).
    Panic { message: String },
    /// The internal global rejection limit was reached before
    /// `Runner::iterations` accepted iterations completed. Rejected
    /// iterations do not count toward `Runner::iterations`, but the
    /// runner keeps track of total attempts (accepted + rejected) via
    /// a crate-private constant so a generator that always rejects
    /// still terminates.
    TooManyRejections {
        rejected_iterations: usize,
        last_reject_location: &'static Location<'static>,
    },
    /// An accepted targeted case finished without calling
    /// [`TestCaseContext::maximize`](crate::TestCaseContext::maximize).
    MissingFeedback,
    /// An accepted targeted case reported `NaN` or infinity via
    /// [`TestCaseContext::maximize`](crate::TestCaseContext::maximize).
    InvalidFeedback,
}

impl Error {
    pub(crate) fn from_panic(
        seed: u64,
        case_index: usize,
        iterations: usize,
        message: String,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        targeted: bool,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            iterations,
            ErrorKind::Panic { message },
            generated,
            stats,
            targeted,
        )
    }

    #[expect(clippy::too_many_arguments)]
    pub(crate) fn from_too_many_rejections(
        seed: u64,
        case_index: usize,
        iterations: usize,
        rejected_iterations: usize,
        last_reject_location: &'static Location<'static>,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        targeted: bool,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            iterations,
            ErrorKind::TooManyRejections {
                rejected_iterations,
                last_reject_location,
            },
            generated,
            stats,
            targeted,
        )
    }

    pub(crate) fn from_missing_feedback(
        seed: u64,
        case_index: usize,
        iterations: usize,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            iterations,
            ErrorKind::MissingFeedback,
            generated,
            stats,
            true,
        )
    }

    pub(crate) fn from_invalid_feedback(
        seed: u64,
        case_index: usize,
        iterations: usize,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            iterations,
            ErrorKind::InvalidFeedback,
            generated,
            stats,
            true,
        )
    }

    fn new(
        seed: u64,
        case_index: usize,
        iterations: usize,
        kind: ErrorKind,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        targeted: bool,
    ) -> Self {
        Self {
            seed,
            case_index,
            iterations,
            kind,
            generated,
            stats,
            targeted,
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
    /// is the count of accepted iterations that ran before the runner
    /// gave up (i.e. the index of the iteration that could not be
    /// accepted). For `MissingFeedback` / `InvalidFeedback`, this is
    /// the index of the accepted case whose feedback failed
    /// validation. That case completed without rejection but is not
    /// counted in `Stats::accepted_iterations`.
    pub fn case_index(&self) -> usize {
        self.case_index
    }

    /// The generated values recorded during the failing case, in call
    /// order. This is a debugging trace, not a stack backtrace.
    pub fn generated(&self) -> &[GeneratedValue] {
        &self.generated
    }

    /// Observability counters accumulated up to (and including) the
    /// failing case. `accepted_iterations` matches
    /// [`case_index`](Self::case_index).
    pub fn stats(&self) -> Stats {
        self.stats
    }
}

impl Error {
    /// The reproduce command shared by
    /// [`Debug`](std::fmt::Debug) and [`Display`](std::fmt::Display).
    /// Reuses the original iteration budget so reruns hit the same
    /// rejection cap. In targeted mode the closure body is a
    /// placeholder: the caller substitutes the original property
    /// closure.
    fn reproduce_command(&self) -> String {
        if self.targeted {
            format!(
                "noprop::Runner::new({:#018x}, {}).run_targeted(|ctx| ...)",
                self.seed, self.iterations
            )
        } else {
            format!(
                "noprop::Runner::new({:#018x}, {})",
                self.seed, self.iterations
            )
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Error {{")?;
        writeln!(f, "    seed: {:#018x},", self.seed)?;
        writeln!(f, "    case_index: {},", self.case_index)?;
        match &self.kind {
            ErrorKind::Panic { message } => {
                writeln!(f, "    panic: {message:?},")?;
            }
            ErrorKind::TooManyRejections {
                rejected_iterations,
                last_reject_location,
            } => {
                writeln!(
                    f,
                    "    too_many_rejections: {{ rejected: {rejected_iterations}, last_reject_at: {}:{} }},",
                    last_reject_location.file(),
                    last_reject_location.line(),
                )?;
            }
            ErrorKind::MissingFeedback => {
                writeln!(f, "    missing_feedback: true,")?;
            }
            ErrorKind::InvalidFeedback => {
                writeln!(f, "    invalid_feedback: true,")?;
            }
        }
        writeln!(f, "    reproduce: {},", self.reproduce_command())?;
        writeln!(
            f,
            "    stats: {{ accepted: {}, rejected: {}, total_samples: {} }},",
            self.stats.accepted_iterations,
            self.stats.rejected_iterations,
            self.stats.total_samples,
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

impl std::fmt::Display for Error {
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
                rejected_iterations,
                last_reject_location,
            } => {
                writeln!(
                    f,
                    "noprop too many rejections at case {} (seed={:#018x}): \
                     {rejected_iterations} rejected iteration(s), last reject at {}:{}",
                    self.case_index,
                    self.seed,
                    last_reject_location.file(),
                    last_reject_location.line(),
                )?;
            }
            ErrorKind::MissingFeedback => {
                writeln!(
                    f,
                    "noprop missing feedback at case {} (seed={:#018x}): \
                     an accepted targeted case never called TestCaseContext::maximize",
                    self.case_index, self.seed,
                )?;
            }
            ErrorKind::InvalidFeedback => {
                writeln!(
                    f,
                    "noprop invalid feedback at case {} (seed={:#018x}): \
                     TestCaseContext::maximize received NaN or infinity",
                    self.case_index, self.seed,
                )?;
            }
        }
        writeln!(f, "reproduce with: {}", self.reproduce_command())?;
        writeln!(
            f,
            "stats: accepted={}, rejected={}, total_samples={}",
            self.stats.accepted_iterations,
            self.stats.rejected_iterations,
            self.stats.total_samples,
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

impl std::error::Error for Error {}
