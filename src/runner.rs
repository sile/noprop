//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::rng::is_iteration_rejected;
use crate::{Error, Result, TestCaseContext};

/// Observability data from a [`Runner::run`](Runner::run) invocation.
///
/// Read from a [`Runner`] after [`run`](Runner::run) returns via
/// [`Runner::stats`](Runner::stats), and also embedded in [`Error`] on
/// failure so the caller can see how far the run progressed before it
/// failed. All three counters are cumulative over the whole `run` call
/// (across every case, accepted or rejected).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Number of iterations whose closure completed without calling
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case). On
    /// a successful `Runner::run`, this equals
    /// `iterations`. On failure, it is
    /// the number of iterations that passed before the failing one
    /// (equivalent to [`Error::case_index`](Error::case_index)).
    pub accepted_iterations: usize,
    /// Total number of iterations discarded via
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case), including
    /// exhausted [`sample_with_rejection`](crate::sample_with_rejection)
    /// helpers (they discard via `reject_case` internally, so the two
    /// origins share this single counter).
    pub rejected_iterations: usize,
    /// Total number of top-level `sample_*` invocations across every
    /// case that ran. Counted per call to the primitive generator
    /// (`sample_u32`, `sample_choice`, `sample_string`, …), not per
    /// underlying byte read or dedup entry — a `sample_char` invocation
    /// that internally retries its 21-bit mask still counts as one
    /// sample. Includes samples produced by rejected iterations, since
    /// those iterations still consumed generator budget.
    pub total_samples: usize,
}

/// A property-based test runner.
///
/// Construct it with [`Runner::new`] and call [`run`](Runner::run):
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner::new(0xDEAD_BEEF, 16).run(|ctx| {
///     let x = noprop::sample_u32(ctx);
///     assert_eq!(x, x);
///     Ok(())
/// });
/// ```
///
/// A constructor is used (instead of struct-literal construction) so
/// that runner-wide configuration (default rejection budget, feedback
/// mode, snapshot directory, …) can be added later without breaking
/// existing call sites. Observability data ([`Stats`]) is exposed via
/// [`Runner::stats`] after `run` returns.
///
/// Other PBT libraries call the iteration count `cases` (proptest),
/// `examples` (Hypothesis), or `tests` (QuickCheck). noprop uses
/// `iterations` for a direct match with the Rust `Iterator` /
/// benchmark vocabulary and to avoid visual confusion with `#[test]`.
///
/// # Configuring seed and iterations
///
/// [`Runner::new`] takes `seed` and `iterations` as required arguments and
/// does not prescribe how to obtain them. A common setup reads both
/// from project-specific environment variables so that failures are
/// reproducible from a failure report (via the seed) and the iteration
/// count can differ between local and CI runs. Use
/// [`seed_from_env_or_time`](crate::seed_from_env_or_time) and
/// [`iterations_from_env`](crate::iterations_from_env) for the two
/// standard lookups:
///
/// ```
/// # fn body() -> Result<(), Box<dyn std::error::Error>> {
/// let seed = noprop::seed_from_env_or_time("MYAPP_SEED")?;
/// let iterations = noprop::iterations_from_env("MYAPP_ITERATIONS", 256)?;
/// noprop::Runner::new(seed, iterations).run(|_ctx| {
///     // property
///     Ok(())
/// })?;
/// # Ok(()) }
/// # body().unwrap();
/// ```
///
/// The env var names shown above are project-specific placeholders;
/// pick names that fit the calling project. Both helpers surface a
/// [`ConfigError`](crate::ConfigError) — via `?` — when the variable
/// is set to something that cannot be parsed, so a mistyped
/// `MYAPP_SEED=hello` fails loudly instead of silently reverting to the
/// clock-derived fallback.
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
/// let _: noprop::Result<()> = noprop::Runner::new(0, 1).run(|_ctx| {
///     let _n: u32 = "42".parse()?;   // ParseIntError -> Box<dyn Error>
///     Ok(())
/// });
/// ```
///
/// Ad-hoc messages work via `Into`:
///
/// ```
/// let _: noprop::Result<()> = noprop::Runner::new(0, 1).run(|_ctx| {
///     if false { return Err("something bad".into()); }
///     Ok(())
/// });
/// ```
///
/// [`Error`]: std::error::Error
pub struct Runner {
    seed: u64,
    iterations: usize,
    stats: Stats,
}

/// Global rejection limit for a single [`Runner::run`] invocation.
///
/// Total rejected iterations (across all iteration indices) are capped
/// so that a generator which always calls
/// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case) still terminates in
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
    /// Construct a runner that invokes the property closure `iterations`
    /// times against a [`TestCaseContext`] seeded with `seed`.
    ///
    /// The number of *accepted* iterations to invoke the closure for.
    ///
    /// An iteration is "accepted" when the closure reaches a verdict
    /// (`Ok(())` / `Err` / panic) without calling
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case)
    /// (directly or via
    /// [`sample_with_rejection`](crate::sample_with_rejection)). Rejected
    /// iterations are retried and are *not* counted toward this budget.
    ///
    /// Rejected iterations are still bounded — the runner enforces an
    /// internal global limit on the total number of rejections it will
    /// tolerate across the whole [`run`](Runner::run) invocation, so a
    /// generator that always rejects still terminates with a
    /// `TooManyRejections` failure instead of looping forever. The
    /// initial limit is a crate-private constant that scales with
    /// `iterations`; there is no public knob for it yet.
    pub fn new(seed: u64, iterations: usize) -> Self {
        Self {
            seed,
            iterations,
            stats: Stats::default(),
        }
    }

    /// Observability counters from the most recent [`run`](Runner::run)
    /// call on this runner. Returns [`Stats::default`] (all zeros)
    /// before `run` has been invoked.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Invoke `f(&mut ctx)` up to `iterations` times against a shared
    /// [`TestCaseContext`] seeded with `seed`.
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
    /// A call to [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case) (either
    /// directly or via
    /// [`sample_with_rejection`](crate::sample_with_rejection)
    /// exhaustion) discards the current iteration, does not count it
    /// toward `iterations`, and retries. A stored rejection state
    /// wins over the closure's own `Ok` / `Err` / non-marker panic
    /// outcome, so user code cannot swallow rejection by catching the
    /// private control-flow marker and returning normally. Total
    /// rejections are bounded — see
    /// `iterations`.
    ///
    /// # Property purity
    ///
    /// The closure is bound as `Fn`, not `FnMut`, so it cannot capture
    /// enclosing variables by mutable reference. Property tests are
    /// meant to be pure functions of the `TestCaseContext`-derived input: keeping
    /// mutation off the closure's captures makes each iteration
    /// independent and each failure reproducible from the seed alone.
    ///
    /// If a test genuinely needs shared state (a debug counter, a
    /// cache, a report sink), reach for interior mutability
    /// (`std::cell::Cell` / `std::cell::RefCell` / atomics) so the
    /// escape from purity is spelled out in the code rather than
    /// hidden behind an unassuming `let mut`.
    pub fn run<F>(&mut self, f: F) -> Result<()>
    where
        F: Fn(&mut TestCaseContext) -> std::result::Result<(), Box<dyn std::error::Error>>,
    {
        self.stats = Stats::default();
        let mut ctx = TestCaseContext::new(self.seed);
        ctx.set_inside_runner();
        let rejection_cap = rejection_limit(self.iterations);
        let mut accepted: usize = 0;
        let mut rejected: usize = 0;

        while accepted < self.iterations {
            ctx.clear_generated();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut ctx)));
            let rejection = ctx.take_rejection();

            if let Some(state) = rejection {
                // Rejection wins over any closure outcome. If the
                // outcome is a non-marker user panic, drop it silently
                // — the "user cannot swallow rejection" guarantee is
                // symmetric: user cannot escalate rejection into a
                // property failure either.
                let _ = outcome;
                rejected += 1;
                if rejected > rejection_cap {
                    self.stats = Stats {
                        accepted_iterations: accepted,
                        rejected_iterations: rejected,
                        total_samples: ctx.total_samples(),
                    };
                    let generated = ctx.take_generated();
                    return Err(Error::from_too_many_rejections(
                        self.seed,
                        accepted,
                        rejected,
                        state.location,
                        generated,
                        self.stats,
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
                            self.stats = Stats {
                                accepted_iterations: accepted,
                                rejected_iterations: rejected,
                                total_samples: ctx.total_samples(),
                            };
                            let generated = ctx.take_generated();
                            let unknown_location = std::panic::Location::caller();
                            return Err(Error::from_too_many_rejections(
                                self.seed,
                                accepted,
                                rejected,
                                unknown_location,
                                generated,
                                self.stats,
                            ));
                        }
                        continue;
                    }
                    panic_message(panic)
                }
            };
            self.stats = Stats {
                accepted_iterations: accepted,
                rejected_iterations: rejected,
                total_samples: ctx.total_samples(),
            };
            let generated = ctx.take_generated();
            return Err(Error::from_panic(
                self.seed,
                accepted,
                message,
                generated,
                self.stats,
            ));
        }
        self.stats = Stats {
            accepted_iterations: accepted,
            rejected_iterations: rejected,
            total_samples: ctx.total_samples(),
        };
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
