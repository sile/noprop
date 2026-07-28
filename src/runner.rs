//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::{Error, Result, Rng};

/// A property-based test runner.
///
/// # Basic usage
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner::new(0xDEAD_BEEF, 16).run(|rng| {
///     let x = noprop::gen_u32(rng);
///     assert_eq!(x, x);
/// });
/// ```
///
/// # Configuring seed and cases
///
/// [`Runner::new`] takes `seed` and `cases` as required arguments and
/// does not prescribe how to obtain them. A common setup reads both
/// from project-specific environment variables, so that failures are
/// reproducible from a failure report (via the seed) and case counts
/// can differ between local and CI runs:
///
/// ```
/// fn seed() -> u64 {
///     std::env::var("MYAPP_SEED")
///         .ok()
///         .and_then(|s| s.parse().ok())
///         .unwrap_or_else(|| {
///             std::time::SystemTime::now()
///                 .duration_since(std::time::UNIX_EPOCH)
///                 .map(|d| d.as_nanos() as u64)
///                 .unwrap_or(0)
///         })
/// }
///
/// fn cases() -> usize {
///     std::env::var("MYAPP_CASES")
///         .ok()
///         .and_then(|s| s.parse().ok())
///         .unwrap_or(256)
/// }
///
/// let _: noprop::Result<()> = noprop::Runner::new(seed(), cases()).run(|_rng| {
///     // property
/// });
/// ```
///
/// The env var names shown above are project-specific placeholders;
/// pick names that fit the calling project.
pub struct Runner {
    seed: u64,
    cases: usize,
}

impl Runner {
    /// Create a runner with the given seed and case count.
    pub fn new(seed: u64, cases: usize) -> Self {
        Self { seed, cases }
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
