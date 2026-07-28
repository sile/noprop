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
///     Ok(())
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
///     Ok(())
/// });
/// ```
///
/// The env var names shown above are project-specific placeholders;
/// pick names that fit the calling project.
///
/// # Failing a case via `Err` or panic
///
/// The property closure signals success by returning `Ok(())`. A
/// failure can be signalled either by returning `Err` or by panicking
/// (typically via `assert!` / `assert_eq!`); both are captured into the
/// resulting [`Error`] uniformly.
///
/// The `Err` variant is `Box<dyn std::error::Error>`, so the `?`
/// operator works for any error type that implements [`Error`]:
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner { seed: 0, cases: 1 }.run(|_rng| {
///     let _n: u32 = "42".parse()?;   // ParseIntError -> Box<dyn Error>
///     Ok(())
/// });
/// ```
///
/// Ad-hoc messages work via `Into`:
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner { seed: 0, cases: 1 }.run(|_rng| {
///     if false { return Err("something bad".into()); }
///     Ok(())
/// });
/// ```
///
/// [`Error`]: std::error::Error
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
    /// Each case invokes `f(&mut rng)`. A returned `Ok(())` counts as a
    /// pass; a returned `Err` or a panic (via `assert!`, `assert_eq!`,
    /// or explicit `panic!`) counts as a failure. Panics are caught by
    /// `catch_unwind`. Either failure mode is wrapped in an [`Error`]
    /// carrying the seed, the failing case index, the failure message,
    /// and the generated-value trace, and returned as `Err`. Subsequent
    /// cases past the first failure are skipped.
    pub fn run<F>(self, mut f: F) -> Result<()>
    where
        F: FnMut(&mut Rng) -> std::result::Result<(), Box<dyn std::error::Error>>,
    {
        let mut rng = Rng::new(self.seed);
        for case_index in 0..self.cases {
            rng.clear_generated();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut rng)));
            let message = match outcome {
                Ok(Ok(())) => continue,
                Ok(Err(err)) => format!("{err}"),
                Err(panic) => panic_message(panic),
            };
            let generated = rng.take_generated();
            return Err(Error::from_panic(self.seed, case_index, message, generated));
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
