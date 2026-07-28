//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::{Error, Result, Rng};

/// Default number of cases per [`Runner::run`].
const DEFAULT_CASES: usize = 256;

/// A property-based test runner.
///
/// # Examples
///
/// ```no_run
/// #[test]
/// fn round_trip() -> noprop::Result<()> {
///     noprop::Runner::new(0xDEAD_BEEF).run(|rng| {
///         let x = noprop::gen_u32(rng);
///         assert_eq!(x, x);
///     })
/// }
/// ```
pub struct Runner {
    seed: u64,
    cases: usize,
}

impl Runner {
    /// Create a runner seeded with `seed`. Default case count is 256.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            cases: DEFAULT_CASES,
        }
    }

    /// Override the number of cases (default 256).
    pub fn with_cases(mut self, cases: usize) -> Self {
        self.cases = cases;
        self
    }

    /// Run `f` for `cases` iterations against a shared [`Rng`] seeded
    /// with `seed`.
    ///
    /// Each case invokes `f(&mut rng)`. If `f` panics (via `assert!`,
    /// `assert_eq!`, or an explicit `panic!`), the panic is caught by
    /// `catch_unwind`, wrapped in an [`Error`] carrying the seed and
    /// the failing case index, and returned as `Err`. Subsequent cases
    /// are skipped.
    ///
    /// If every case completes without panicking, returns `Ok(())`.
    pub fn run<F>(self, mut f: F) -> Result<()>
    where
        F: FnMut(&mut Rng),
    {
        let mut rng = Rng::new(self.seed);
        for case_index in 0..self.cases {
            let payload = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut rng)));
            if let Err(panic) = payload {
                let message = panic_message(panic);
                return Err(Error::from_panic(self.seed, case_index, message));
            }
        }
        Ok(())
    }
}

/// Extract a human-readable message from a `catch_unwind` payload.
///
/// Panic payloads come in two common shapes (`&'static str` and `String`).
/// Anything else collapses to a placeholder — this is not a typical case,
/// but we don't want to swallow the failure entirely.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}
