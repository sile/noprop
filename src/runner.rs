//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::rng::{RejectionState, is_iteration_rejected};
use crate::{RunError, RunResult, TestCaseContext, TestResult};

/// Observability data from a [`Runner::run`](Runner::run) invocation.
///
/// Read from a [`Runner`] after the run returns via
/// [`Runner::stats`](Runner::stats), and also embedded in
/// [`RunError`](crate::RunError) on failure so the caller can see how far
/// the run progressed before it failed. All counters are cumulative over
/// the whole run (across every case, accepted or rejected).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Number of cases whose closure completed without calling
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case). On
    /// a successful run, this equals
    /// `cases`. On failure, it is
    /// the number of cases that passed before the failing one
    /// (equivalent to [`RunError::case_index`](RunError::case_index)).
    pub accepted_cases: usize,
    /// Total number of cases discarded via
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case), including
    /// exhausted [`sample_with_rejection`](crate::sample_with_rejection)
    /// helpers (they discard via `reject_case` internally, so the two
    /// origins share this single counter).
    pub rejected_cases: usize,
    /// Total number of top-level `sample_*` invocations across every
    /// case that ran. Counted per call to the primitive generator
    /// (`sample_u32`, `sample_choice`, `sample_string`, …), not per
    /// underlying byte read or dedup entry — a `sample_char` invocation
    /// that internally retries its 21-bit mask still counts as one
    /// sample. Includes samples produced by rejected cases, since
    /// those cases still consumed generator budget.
    pub total_samples: usize,
}

/// A property-based test runner.
///
/// Construct it with [`Runner::new`] and call [`run`](Runner::run):
///
/// ```
/// let _: noprop::RunResult = noprop::Runner::new(0xDEAD_BEEF).run(16, |ctx| {
///     let x = noprop::sample_u32(ctx);
///     assert_eq!(x, x);
///     Ok(())
/// });
/// ```
///
/// A constructor is used (instead of struct-literal construction) so
/// that runner-wide configuration (default rejection budget, …) can be
/// added later without breaking existing call sites. Observability data
/// ([`Stats`]) is exposed via [`Runner::stats`] after `run` returns.
///
/// # Configuring the seed
///
/// [`Runner::new`] takes `seed` as a required argument and
/// does not prescribe how to obtain it. A common setup reads it
/// from a project-specific environment variable so that failures are
/// reproducible from a failure report (via the seed). Use
/// [`seed_from_env_or_time`](crate::seed_from_env_or_time) for the
/// standard lookup:
///
/// ```
/// # fn body() -> noprop::TestResult {
/// let seed = noprop::seed_from_env_or_time("MYAPP_SEED")?;
/// noprop::Runner::new(seed).run(256, |_ctx| {
///     // property
///     Ok(())
/// })?;
/// Ok(())
/// }
/// # body().unwrap();
/// ```
///
/// The env var name shown above is a project-specific placeholder;
/// pick a name that fits the calling project. The helper accepts
/// decimal values and `0x`-prefixed hex values with optional `_`
/// separators, so the hex seed printed by a failure report can be
/// pasted into the environment variable directly. The
/// helper surfaces a
/// boxed error — via `?` — when the variable
/// is set to something that cannot be parsed, so a mistyped
/// `MYAPP_SEED=hello` fails loudly instead of silently reverting to the
/// clock-derived fallback.
///
/// # Failing a case via `Err` or panic
///
/// The property closure signals success by returning `Ok(())`. A
/// failure can be signalled either by returning `Err` or by panicking
/// (typically via `assert!` / `assert_eq!`); both are captured into the
/// resulting [`RunError`](crate::RunError) uniformly.
///
/// The `Err` variant is `Box<dyn std::error::Error>`, so the `?`
/// operator works for any error type that implements [`Error`]:
///
/// ```
/// let _: noprop::RunResult = noprop::Runner::new(0).run(1, |_ctx| {
///     let _n: u32 = "42".parse()?;   // ParseIntError -> Box<dyn Error>
///     Ok(())
/// });
/// ```
///
/// Ad-hoc messages work via `Into`:
///
/// ```
/// let _: noprop::RunResult = noprop::Runner::new(0).run(1, |_ctx| {
///     if false { return Err("something bad".into()); }
///     Ok(())
/// });
/// ```
///
/// [`Error`]: std::error::Error
pub struct Runner {
    seed: u64,
    stats: Stats,
}

/// Global rejection limit for a single [`Runner::run`](Runner::run)
/// invocation.
///
/// Total rejected cases (across all case indices) are capped
/// so that a generator which always calls
/// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case) still terminates in
/// finite time with a `TooManyRejections` failure.
///
/// Scaled with `cases` so that a generous case budget also
/// gets a generous rejection budget, with a floor for very small
/// `cases` (including `0`). The concrete formula and floor are
/// deliberately kept crate-private; both are subject to change once
/// real-world usage produces measurement data.
fn rejection_limit(cases: usize) -> usize {
    const FLOOR: usize = 1024;
    FLOOR.max(cases.saturating_mul(10))
}

impl Runner {
    /// Construct a runner that invokes the property closure against a
    /// [`TestCaseContext`] seeded with `seed`.
    ///
    /// For the usual "read the seed from an environment variable, with a
    /// clock-derived fallback" setup, see
    /// [`seed_from_env_or_time`](crate::seed_from_env_or_time).
    ///
    /// The number of *accepted* cases to invoke the closure for is
    /// given per run, via [`run`](Runner::run).
    ///
    /// A case is "accepted" when the closure reaches a verdict
    /// (`Ok(())` / `Err` / panic) without calling
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case)
    /// (directly or via
    /// [`sample_with_rejection`](crate::sample_with_rejection)). Rejected
    /// cases are retried and are *not* counted toward the budget.
    ///
    /// Rejected cases are still bounded — the runner enforces an
    /// internal global limit on the total number of rejections it will
    /// tolerate across the whole [`run`](Runner::run) invocation, so a
    /// generator that always rejects still terminates with a
    /// `TooManyRejections` failure instead of looping forever. The
    /// initial limit is a crate-private constant that scales with
    /// `cases`; there is no public knob for it yet.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            stats: Stats::default(),
        }
    }

    /// Observability counters from the most recent [`run`](Runner::run)
    /// call on this runner. Returns [`Stats::default`] (all zeros)
    /// before a run has been invoked.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Invoke `f(&mut ctx)` up to `cases` times against a shared
    /// [`TestCaseContext`] seeded with `seed`.
    ///
    /// Each invocation is one property case. A returned `Ok(())`
    /// counts as a pass; a returned `Err` or a panic (via `assert!`,
    /// `assert_eq!`, or explicit `panic!`) counts as a failure. Panics
    /// are caught by `catch_unwind`. Either failure mode is wrapped in
    /// a [`RunError`](crate::RunError) carrying the seed, the failing
    /// case's index,
    /// the failure message, and the generated-value trace, and returned
    /// as `Err`. Subsequent cases past the first failure are
    /// skipped.
    ///
    /// A call to [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case) (either
    /// directly or via
    /// [`sample_with_rejection`](crate::sample_with_rejection)
    /// exhaustion) discards the current case, does not count it
    /// toward `cases`, and retries. A stored rejection state
    /// wins over the closure's own `Ok` / `Err` / non-marker panic
    /// outcome, so user code cannot swallow rejection by catching the
    /// private control-flow marker and returning normally. Total
    /// rejections are bounded — see
    /// `cases`.
    ///
    /// # Property purity
    ///
    /// The closure is bound as `Fn`, not `FnMut`, so it cannot capture
    /// enclosing variables by mutable reference. Property tests are
    /// meant to be pure functions of the `TestCaseContext`-derived input: keeping
    /// mutation off the closure's captures makes each case
    /// independent and each failure reproducible from the seed alone.
    ///
    /// If a test genuinely needs shared state (a debug counter, a
    /// cache, a report sink), reach for interior mutability
    /// (`std::cell::Cell` / `std::cell::RefCell` / atomics) so the
    /// escape from purity is spelled out in the code rather than
    /// hidden behind an unassuming `let mut`.
    pub fn run<F>(&mut self, cases: usize, f: F) -> RunResult
    where
        F: Fn(&mut TestCaseContext) -> TestResult,
    {
        self.stats = Stats::default();
        let mut ctx = TestCaseContext::new(self.seed);
        ctx.set_inside_runner();
        let rejection_cap = rejection_limit(cases);
        let mut accepted: usize = 0;
        let mut rejected: usize = 0;

        while accepted < cases {
            ctx.clear_generated();
            match run_case(&f, &mut ctx) {
                CaseVerdict::Rejected(state) => {
                    rejected += 1;
                    if rejected > rejection_cap {
                        record_stats(self, accepted, rejected, ctx.total_samples());
                        let generated = ctx.take_generated();
                        return Err(RunError::from_too_many_rejections(
                            self.seed,
                            cases,
                            state.location,
                            generated,
                            self.stats,
                        ));
                    }
                    continue;
                }
                CaseVerdict::Completed(CaseOutcome::Passed) => {
                    accepted += 1;
                    continue;
                }
                CaseVerdict::Completed(CaseOutcome::Failed(message)) => {
                    record_stats(self, accepted, rejected, ctx.total_samples());
                    let generated = ctx.take_generated();
                    return Err(RunError::from_panic(
                        self.seed, cases, message, generated, self.stats,
                    ));
                }
            }
        }
        record_stats(self, accepted, rejected, ctx.total_samples());
        Ok(())
    }
}

/// Human-oriented summary of the runner's seed and the most recent
/// run's observability counters, for embedding in assertion messages.
///
/// The exact string format is not part of the API contract; machine
/// checks should read [`Runner::stats`](Runner::stats) instead.
impl std::fmt::Display for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "noprop::Runner {{ seed: {:#018x}, stats: {{ accepted: {}, rejected: {}, total_samples: {} }} }}",
            self.seed,
            self.stats.accepted_cases,
            self.stats.rejected_cases,
            self.stats.total_samples,
        )
    }
}

/// The closure's own verdict for one property case.
enum CaseOutcome {
    /// The closure returned `Ok(())` without rejecting.
    Passed,
    /// The closure returned `Err` or panicked; the run must fail with
    /// this message.
    Failed(String),
}

/// The runner-side verdict for one property case, after any stored
/// rejection has been resolved.
enum CaseVerdict {
    /// The iteration was rejected (`reject_case`, or a stray
    /// `IterationRejected` marker without stored state).
    Rejected(RejectionState),
    /// The closure finished without rejecting.
    Completed(CaseOutcome),
}

/// Invoke the property closure once and classify the verdict.
///
/// A stored rejection wins over any closure outcome: a non-marker user
/// panic raised alongside a rejection is dropped, so user code can
/// neither swallow rejection nor escalate it into a property failure.
fn run_case<F>(f: &F, ctx: &mut TestCaseContext) -> CaseVerdict
where
    F: Fn(&mut TestCaseContext) -> TestResult,
{
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(ctx)));
    if let Some(state) = ctx.take_rejection() {
        return CaseVerdict::Rejected(state);
    }
    let message = match outcome {
        Ok(Ok(())) => return CaseVerdict::Completed(CaseOutcome::Passed),
        Ok(Err(err)) => format!("{err}"),
        Err(panic) => {
            // Defensive: a stray IterationRejected marker without a
            // stored rejection state shouldn't happen because
            // `reject_case` always sets the state before resuming
            // unwind. If it somehow does, treat it as rejection rather
            // than as a property failure with an opaque payload.
            if is_iteration_rejected(&*panic) {
                let location = std::panic::Location::caller();
                return CaseVerdict::Rejected(RejectionState { location });
            }
            panic_message(panic)
        }
    };
    CaseVerdict::Completed(CaseOutcome::Failed(message))
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

/// Record the run progress counters on the runner. Every exit path
/// (success, property failure, rejection cap) reports the same counters.
fn record_stats(runner: &mut Runner, accepted: usize, rejected: usize, total_samples: usize) {
    runner.stats = Stats {
        accepted_cases: accepted,
        rejected_cases: rejected,
        total_samples,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_cap_accepts_exactly_at_the_boundary() {
        // The runner fires TooManyRejections when `rejected >
        // rejection_cap`, so the last non-fatal state is `rejected
        // == cap`. Reject exactly `rejection_limit(cases)` times and
        // then accept once; the run must succeed with
        // `rejected == cap`, showing the boundary is inclusive on
        // the success side (fixes a coverage gap - only `> cap` was
        // previously tested).
        let cases = 1;
        let cap = rejection_limit(cases);
        let count = std::cell::Cell::new(0usize);
        let mut runner = Runner::new(1);
        runner
            .run(cases, |ctx| {
                let n = count.get();
                count.set(n + 1);
                if n < cap {
                    ctx.reject_case();
                }
                Ok(())
            })
            .expect("rejecting exactly rejection_limit(cases) times must not exceed the cap");
        let stats = runner.stats();
        assert_eq!(stats.accepted_cases, cases);
        assert_eq!(stats.rejected_cases, cap);
    }
}
