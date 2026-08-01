//! Error and result types for [`Runner::run`](crate::Runner::run).

use std::panic::Location;

use crate::GeneratedValue;

/// Result alias used across noprop's public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure information from a [`Runner::run`](crate::Runner::run) invocation.
///
/// A property failure (panic or returned `Err`) is deterministically
/// reproducible from `seed()` and `case_index()`: rerunning
/// `noprop::Runner { seed: err.seed(), .. }` with at least
/// `err.case_index() + 1` iterations will hit the same failure again.
///
/// A `TooManyRejections` failure — raised when
/// [`Rng::reject_case`](crate::Rng::reject_case) fires so often that
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
/// reproduce with: noprop::Runner { seed: 0x..., iterations: N }
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
}

impl Error {
    pub(crate) fn from_panic(
        seed: u64,
        case_index: usize,
        message: String,
        generated: Vec<GeneratedValue>,
    ) -> Self {
        Self {
            seed,
            case_index,
            kind: ErrorKind::Panic { message },
            generated,
        }
    }

    pub(crate) fn from_too_many_rejections(
        seed: u64,
        case_index: usize,
        rejected_iterations: usize,
        last_reject_location: &'static Location<'static>,
        generated: Vec<GeneratedValue>,
    ) -> Self {
        Self {
            seed,
            case_index,
            kind: ErrorKind::TooManyRejections {
                rejected_iterations,
                last_reject_location,
            },
            generated,
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
        }
        writeln!(
            f,
            "    reproduce: noprop::Runner {{ seed: {:#018x}, iterations: {} }},",
            self.seed,
            self.reproduce_iterations(),
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
        }
        writeln!(
            f,
            "reproduce with: noprop::Runner {{ seed: {:#018x}, iterations: {} }}",
            self.seed,
            self.reproduce_iterations(),
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
