//! Error and result types for [`Runner::run`](crate::Runner::run).

/// Result alias used across noprop's public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure information from a [`Runner::run`](crate::Runner::run) invocation.
///
/// A failure is deterministically reproducible from `seed()` and
/// `case_index()`: rerunning `noprop::Runner::new(err.seed())` with at
/// least `err.case_index() + 1` cases will hit the same failure again.
///
/// The `Debug` and `Display` output includes the panic message captured
/// from the user's closure, so returning this from a `#[test]` function
/// prints a self-contained failure report through the standard test
/// harness.
pub struct Error {
    seed: u64,
    case_index: usize,
    kind: ErrorKind,
}

enum ErrorKind {
    /// The property closure panicked in this case (typically via
    /// `assert!` / `assert_eq!` or an explicit `panic!`).
    Panic { message: String },
}

impl Error {
    pub(crate) fn from_panic(seed: u64, case_index: usize, message: String) -> Self {
        Self {
            seed,
            case_index,
            kind: ErrorKind::Panic { message },
        }
    }

    /// The seed that was passed to [`Runner::new`](crate::Runner::new).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The zero-based index of the case that failed.
    pub fn case_index(&self) -> usize {
        self.case_index
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ErrorKind::Panic { message } = &self.kind;
        f.debug_struct("Error")
            .field("seed", &format_args!("{:#018x}", self.seed))
            .field("case_index", &self.case_index)
            .field("panic", message)
            .finish()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ErrorKind::Panic { message } = &self.kind;
        write!(
            f,
            "case {} (seed={:#018x}) panicked: {}",
            self.case_index, self.seed, message
        )
    }
}

impl std::error::Error for Error {}
