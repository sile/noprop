//! Error and result types for [`Runner::run`](crate::Runner::run).

use std::panic::Location;

use crate::GeneratedValue;
use crate::runner::Stats;

/// Result alias used across noprop's public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure information from a [`Runner::run`](crate::Runner::run) invocation.
///
/// A property failure (panic or returned `Err`) is deterministically
/// reproducible from `seed()` and `case_index()`: rerunning
/// `noprop::Runner::new(err.seed(), err.case_index() + 1)` will hit
/// the same failure again.
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
/// Both formats also print a copy-pasteable
///
/// ```text
/// reproduce with: noprop::Runner::new(0x..., N)
/// ```
///
/// line where `iterations = case_index + 1`, so re-triggering the
/// same failure does not require the user to compute the minimum
/// re-run size by hand.
pub struct Error {
    seed: u64,
    case_index: usize,
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
        message: String,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            ErrorKind::Panic { message },
            generated,
            stats,
            false,
        )
    }

    pub(crate) fn from_panic_targeted(
        seed: u64,
        case_index: usize,
        message: String,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            ErrorKind::Panic { message },
            generated,
            stats,
            true,
        )
    }

    pub(crate) fn from_too_many_rejections(
        seed: u64,
        case_index: usize,
        rejected_iterations: usize,
        last_reject_location: &'static Location<'static>,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            ErrorKind::TooManyRejections {
                rejected_iterations,
                last_reject_location,
            },
            generated,
            stats,
            false,
        )
    }

    pub(crate) fn from_too_many_rejections_targeted(
        seed: u64,
        case_index: usize,
        rejected_iterations: usize,
        last_reject_location: &'static Location<'static>,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            ErrorKind::TooManyRejections {
                rejected_iterations,
                last_reject_location,
            },
            generated,
            stats,
            true,
        )
    }

    pub(crate) fn from_missing_feedback(
        seed: u64,
        case_index: usize,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            ErrorKind::MissingFeedback,
            generated,
            stats,
            true,
        )
    }

    pub(crate) fn from_invalid_feedback(
        seed: u64,
        case_index: usize,
        generated: Vec<GeneratedValue>,
        stats: Stats,
    ) -> Self {
        Self::new(
            seed,
            case_index,
            ErrorKind::InvalidFeedback,
            generated,
            stats,
            true,
        )
    }

    fn new(
        seed: u64,
        case_index: usize,
        kind: ErrorKind,
        generated: Vec<GeneratedValue>,
        stats: Stats,
        targeted: bool,
    ) -> Self {
        Self {
            seed,
            case_index,
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
    /// failing case. `accepted_iterations` matches
    /// [`case_index`](Self::case_index).
    pub fn stats(&self) -> Stats {
        self.stats
    }
}

impl Error {
    /// Number of `iterations` the caller needs to reproduce this
    /// failure — always `case_index() + 1`. Split out so both
    /// [`Debug`](std::fmt::Debug) and [`Display`](std::fmt::Display)
    /// share the same computation and format.
    fn reproduce_iterations(&self) -> usize {
        self.case_index + 1
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
        if self.targeted {
            writeln!(
                f,
                "    reproduce: noprop::Runner::new({:#018x}, {}).run_targeted(|ctx| ...),",
                self.seed,
                self.reproduce_iterations(),
            )?;
        } else {
            writeln!(
                f,
                "    reproduce: noprop::Runner::new({:#018x}, {}),",
                self.seed,
                self.reproduce_iterations(),
            )?;
        }
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
        if self.targeted {
            writeln!(
                f,
                "reproduce with: noprop::Runner::new({:#018x}, {}).run_targeted(|ctx| ...)",
                self.seed,
                self.reproduce_iterations(),
            )?;
        } else {
            writeln!(
                f,
                "reproduce with: noprop::Runner::new({:#018x}, {})",
                self.seed,
                self.reproduce_iterations(),
            )?;
        }
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
