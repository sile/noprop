//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::rng::is_iteration_rejected;
use crate::{Error, Result, Rng};

/// A property-based test runner.
///
/// [`Runner`] is a small config struct with public fields. Construct it
/// with a struct literal and call [`run`](Runner::run):
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner { seed: 0xDEAD_BEEF, iterations: 16 }.run(|rng| {
///     let x = noprop::sample_u32(rng);
///     assert_eq!(x, x);
///     Ok(())
/// });
/// ```
///
/// Named-field construction avoids the two-numeric-args swap risk of a
/// positional `new(seed, iterations)`.
///
/// Other PBT libraries call the same count `cases` (proptest),
/// `examples` (Hypothesis), or `tests` (QuickCheck). noprop uses
/// `iterations` for a direct match with the Rust `Iterator` /
/// benchmark vocabulary and to avoid visual confusion with `#[test]`.
///
/// # Configuring seed and iterations
///
/// [`Runner`] takes `seed` and `iterations` as required fields and does
/// not prescribe how to obtain them. A common setup reads both from
/// project-specific environment variables so that failures are
/// reproducible from a failure report (via the seed) and the iteration
/// count can differ between local and CI runs:
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
/// fn iterations() -> usize {
///     std::env::var("MYAPP_ITERATIONS")
///         .ok()
///         .and_then(|s| s.parse().ok())
///         .unwrap_or(256)
/// }
///
/// let _: noprop::Result<()> = noprop::Runner { seed: seed(), iterations: iterations() }
///     .run(|_rng| {
///         // property
///         Ok(())
///     });
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
/// let _: noprop::Result<()> = noprop::Runner { seed: 0, iterations: 1 }.run(|_rng| {
///     let _n: u32 = "42".parse()?;   // ParseIntError -> Box<dyn Error>
///     Ok(())
/// });
/// ```
///
/// Ad-hoc messages work via `Into`:
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner { seed: 0, iterations: 1 }.run(|_rng| {
///     if false { return Err("something bad".into()); }
///     Ok(())
/// });
/// ```
///
/// [`Error`]: std::error::Error
pub struct Runner {
    /// The seed used to construct the internal [`Rng`].
    pub seed: u64,
    /// The number of *accepted* iterations to invoke the closure for.
    ///
    /// An iteration is "accepted" when the closure reaches a verdict
    /// (`Ok(())` / `Err` / panic) without calling
    /// [`Rng::reject_case`](crate::Rng::reject_case) (directly or via
    /// [`sample_with_rejection`](crate::sample_with_rejection)).
    /// Rejected iterations are retried and are *not* counted toward
    /// this budget.
    ///
    /// Rejected iterations are still bounded — the runner enforces an
    /// internal global limit on the total number of rejections it will
    /// tolerate across the whole [`run`](Runner::run) invocation, so a
    /// generator that always rejects still terminates with a
    /// `TooManyRejections` failure instead of looping forever. The
    /// initial limit is a crate-private constant that scales with
    /// `iterations`; there is no public knob for it yet.
    pub iterations: usize,
}

/// Global rejection limit for a single [`Runner::run`] invocation.
///
/// Total rejected iterations (across all iteration indices) are capped
/// so that a generator which always calls
/// [`Rng::reject_case`](crate::Rng::reject_case) still terminates in
/// finite time with a `TooManyRejections` failure.
///
/// Scaled with `iterations` so that a generous iteration budget also
/// gets a generous rejection budget, with a floor for very small
/// `iterations` (including `0`). The concrete formula and floor are
/// deliberately kept crate-private; both are subject to change once
/// real-world usage produces measurement data.
fn rejection_limit(iterations: usize) -> usize {
    const FLOOR: usize = 1024;
    FLOOR.max(iterations.saturating_mul(10))
}

impl Runner {
    /// Invoke `f(&mut rng)` up to `iterations` times against a shared
    /// [`Rng`] seeded with `seed`.
    ///
    /// Each invocation is one property "iteration". A returned `Ok(())`
    /// counts as a pass; a returned `Err` or a panic (via `assert!`,
    /// `assert_eq!`, or explicit `panic!`) counts as a failure. Panics
    /// are caught by `catch_unwind`. Either failure mode is wrapped in
    /// an [`Error`] carrying the seed, the failing iteration's index,
    /// the failure message, and the generated-value trace, and returned
    /// as `Err`. Subsequent iterations past the first failure are
    /// skipped.
    ///
    /// A call to [`Rng::reject_case`](crate::Rng::reject_case) (either
    /// directly or via
    /// [`sample_with_rejection`](crate::sample_with_rejection)
    /// exhaustion) discards the current iteration, does not count it
    /// toward `iterations`, and retries. A stored rejection state
    /// wins over the closure's own `Ok` / `Err` / non-marker panic
    /// outcome, so user code cannot swallow rejection by catching the
    /// private control-flow marker and returning normally. Total
    /// rejections are bounded — see
    /// [`Runner::iterations`](Runner::iterations).
    ///
    /// # Property purity
    ///
    /// The closure is bound as `Fn`, not `FnMut`, so it cannot capture
    /// enclosing variables by mutable reference. Property tests are
    /// meant to be pure functions of the `Rng`-derived input: keeping
    /// mutation off the closure's captures makes each iteration
    /// independent and each failure reproducible from the seed alone.
    ///
    /// If a test genuinely needs shared state (a debug counter, a
    /// cache, a report sink), reach for interior mutability
    /// (`std::cell::Cell` / `std::cell::RefCell` / atomics) so the
    /// escape from purity is spelled out in the code rather than
    /// hidden behind an unassuming `let mut`.
    pub fn run<F>(self, f: F) -> Result<()>
    where
        F: Fn(&mut Rng) -> std::result::Result<(), Box<dyn std::error::Error>>,
    {
        let mut rng = Rng::new(self.seed);
        rng.set_inside_runner();
        let rejection_cap = rejection_limit(self.iterations);
        let mut accepted: usize = 0;
        let mut rejected: usize = 0;

        while accepted < self.iterations {
            rng.clear_generated();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut rng)));
            let rejection = rng.take_rejection();

            if let Some(state) = rejection {
                // Rejection wins over any closure outcome. If the
                // outcome is a non-marker user panic, drop it silently
                // — the "user cannot swallow rejection" guarantee is
                // symmetric: user cannot escalate rejection into a
                // property failure either.
                let _ = outcome;
                rejected += 1;
                if rejected > rejection_cap {
                    let generated = rng.take_generated();
                    return Err(Error::from_too_many_rejections(
                        self.seed,
                        accepted,
                        rejected,
                        state.location,
                        generated,
                    ));
                }
                continue;
            }

            let message = match outcome {
                Ok(Ok(())) => {
                    accepted += 1;
                    continue;
                }
                Ok(Err(err)) => format!("{err}"),
                Err(panic) => {
                    // Defensive: an IterationRejected marker without a
                    // stored rejection state shouldn't happen because
                    // `reject_case` always sets the state before
                    // resuming unwind. If it somehow does, treat it as
                    // rejection rather than as a property failure with
                    // an opaque payload.
                    if is_iteration_rejected(&*panic) {
                        rejected += 1;
                        if rejected > rejection_cap {
                            let generated = rng.take_generated();
                            let unknown_location = std::panic::Location::caller();
                            return Err(Error::from_too_many_rejections(
                                self.seed,
                                accepted,
                                rejected,
                                unknown_location,
                                generated,
                            ));
                        }
                        continue;
                    }
                    panic_message(panic)
                }
            };
            let generated = rng.take_generated();
            return Err(Error::from_panic(self.seed, accepted, message, generated));
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
