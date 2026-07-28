//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::{Error, Result, Rng};

/// A property-based test runner.
///
/// [`Runner`] is a small config struct with public fields. Construct it
/// with a struct literal and call [`run`](Runner::run):
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner { seed: 0xDEAD_BEEF, cases: 16 }.run(|rng| {
///     let x = noprop::gen_u32(rng);
///     assert_eq!(x, x);
/// });
/// ```
///
/// Named-field construction avoids the two-numeric-args swap risk of a
/// positional `new(seed, cases)`.
///
/// # Configuring seed and cases
///
/// [`Runner`] takes `seed` and `cases` as required fields and does not
/// prescribe how to obtain them. A common setup reads both from
/// project-specific environment variables so that failures are
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
/// let _: noprop::Result<()> = noprop::Runner { seed: seed(), cases: cases() }.run(|_rng| {
///     // property
/// });
/// ```
///
/// The env var names shown above are project-specific placeholders;
/// pick names that fit the calling project.
pub struct Runner {
    /// The seed used to construct the internal [`Rng`].
    pub seed: u64,
    /// The number of times the closure is invoked in [`run`](Runner::run).
    pub cases: usize,
}

impl Runner {
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
            rng.clear_generated();
            let payload = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut rng)));
            if let Err(panic) = payload {
                let message = panic_message(panic);
                let generated = rng.take_generated();
                return Err(Error::from_panic(self.seed, case_index, message, generated));
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
