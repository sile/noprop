//! Error and result types for [`Runner::run`](crate::Runner::run).

use crate::GeneratedValue;

/// Result alias used across noprop's public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure information from a [`Runner::run`](crate::Runner::run) invocation.
///
/// A failure is deterministically reproducible from `seed()` and
/// `case_index()`: rerunning `noprop::Runner { seed: err.seed(), .. }`
/// with at least `err.case_index() + 1` cases will hit the same failure
/// again.
///
/// `generated()` returns the sequence of values every primitive
/// generator produced during the failing case, together with each call
/// site's source location. This trace is a debugging aid — it is *not*
/// a stack backtrace.
///
/// The `Debug` and `Display` output includes the panic message captured
/// from the user's closure along with the generated-value list, so
/// returning this from a `#[test]` function prints a self-contained
/// failure report through the standard test harness.
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

    /// The seed that was passed to the [`Runner`](crate::Runner) that
    /// produced this failure.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The zero-based index of the case that failed.
    pub fn case_index(&self) -> usize {
        self.case_index
    }

    /// The generated values recorded during the failing case, in call
    /// order. This is a debugging trace, not a stack backtrace.
    pub fn generated(&self) -> &[GeneratedValue] {
        &self.generated
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ErrorKind::Panic { message } = &self.kind;
        writeln!(f, "Error {{")?;
        writeln!(f, "    seed: {:#018x},", self.seed)?;
        writeln!(f, "    case_index: {},", self.case_index)?;
        writeln!(f, "    panic: {message:?},")?;
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
        let ErrorKind::Panic { message } = &self.kind;
        writeln!(
            f,
            "noprop failure at case {} (seed={:#018x}): {}",
            self.case_index, self.seed, message
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
