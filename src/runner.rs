//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::rng::{
    ChoiceMeta, ChoiceSequence, FeedbackState, ScalarFeedback, XoshiroState, is_iteration_rejected,
};
use crate::{Error, Result, TestCaseContext};

/// Maximum number of candidates kept in the targeted corpus (top-k).
///
/// All four search constants (`CORPUS_SIZE`, `MUTATION_DENOM`,
/// `RANDOM_RESTART_DENOM`, `LOW_SCORE_DENOM`) are initial guesses;
/// their tuning is deferred until benchmark data exists.
const CORPUS_SIZE: usize = 64;

/// Denominator of the per-draw mutation probability: one in
/// `MUTATION_DENOM` draws of a selected candidate are rewritten.
const MUTATION_DENOM: u64 = 4;

/// Denominator of the random-restart probability: one in
/// `RANDOM_RESTART_DENOM` candidates are freshly generated instead of
/// mutated from the corpus.
const RANDOM_RESTART_DENOM: u64 = 8;

/// Denominator of the low-score pick probability: one in
/// `LOW_SCORE_DENOM` corpus picks target the single lowest-scored
/// entry, keeping an alternative search path alive.
const LOW_SCORE_DENOM: u64 = 4;

/// Observability data from a [`Runner::run`](Runner::run) or
/// [`Runner::run_targeted`](Runner::run_targeted) invocation.
///
/// Read from a [`Runner`] after the run returns via
/// [`Runner::stats`](Runner::stats), and also embedded in [`Error`] on
/// failure so the caller can see how far the run progressed before it
/// failed. All three counters are cumulative over the whole run
/// (across every case, accepted or rejected).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Number of iterations whose closure completed without calling
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case). On
    /// a successful run, this equals
    /// `iterations`. On failure, it is
    /// the number of iterations that passed before the failing one
    /// (equivalent to [`Error::case_index`](Error::case_index)).
    pub accepted_iterations: usize,
    /// Total number of iterations discarded via
    /// [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case), including
    /// exhausted [`sample_with_rejection`](crate::sample_with_rejection)
    /// helpers (they discard via `reject_case` internally, so the two
    /// origins share this single counter), and — under
    /// [`Runner::run_targeted`](Runner::run_targeted) — the
    /// exploratory-replay draw cap discards.
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

/// Global rejection limit for a single [`Runner::run`](Runner::run) or
/// [`Runner::run_targeted`](Runner::run_targeted) invocation.
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

    /// Observability counters from the most recent
    /// [`run`](Runner::run) or [`run_targeted`](Runner::run_targeted)
    /// call on this runner. Returns [`Stats::default`] (all zeros)
    /// before a run has been invoked.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Targeted search over a scalar "distance to failure" reported via
    /// [`TestCaseContext::maximize`](crate::TestCaseContext::maximize).
    ///
    /// The property closure has the same shape as [`run`](Runner::run),
    /// so the same property can be exercised under both policies. Each
    /// accepted case must call `maximize` with a finite score — a case
    /// that never reports a score (missing feedback) or reports `NaN` /
    /// infinity (invalid feedback) terminates the run with a distinct
    /// [`Error`].
    ///
    /// Candidates are produced from a bounded score corpus: accepted
    /// cases are recorded as choice sequences, mutated within their
    /// recorded constraints, and replayed with exploratory generation
    /// for draws the mutation introduces. With probability
    /// `1 / RANDOM_RESTART_DENOM` a candidate is freshly generated
    /// instead, so the search can escape local optima.
    ///
    /// The rejection semantics (global rejection cap, `Stats`, and the
    /// `Runner::iterations` budget counting only accepted cases) match
    /// [`run`](Runner::run).
    ///
    /// # Example
    ///
    /// ```
    /// let mut runner = noprop::Runner::new(0xDEAD_BEEF, 16);
    /// runner
    ///     .run_targeted(|ctx| {
    ///         let x = noprop::sample_u32(ctx);
    ///         ctx.maximize((x as f64) / u32::MAX as f64);
    ///         Ok(())
    ///     })
    ///     .expect("targeted run must succeed");
    /// ```
    pub fn run_targeted<F>(&mut self, f: F) -> Result<()>
    where
        F: Fn(&mut TestCaseContext) -> std::result::Result<(), Box<dyn std::error::Error>>,
    {
        self.stats = Stats::default();
        let mut search = TargetedSearch::new(self.seed);
        let rejection_cap = rejection_limit(self.iterations);
        let mut accepted: usize = 0;
        let mut rejected: usize = 0;
        let mut total_samples: usize = 0;

        while accepted < self.iterations {
            // Each iteration gets a fresh context (recording or
            // exploratory), so there is no per-case state to clear.
            let mut ctx = search.next_context();
            ctx.set_inside_runner();
            ctx.enable_targeted();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut ctx)));
            let rejection = ctx.take_rejection();
            total_samples += ctx.total_samples();

            if let Some(state) = rejection {
                rejected += 1;
                if rejected > rejection_cap {
                    return Err(too_many_rejections(
                        self,
                        &mut ctx,
                        accepted,
                        rejected,
                        total_samples,
                        state.location,
                    ));
                }
                continue;
            }

            let message = match outcome {
                Ok(Ok(())) => {
                    // Accepted case: feedback is mandatory in targeted mode.
                    let feedback = ctx.take_feedback();
                    let score = match feedback {
                        FeedbackState::Targeted { max_score } => match max_score {
                            ScalarFeedback::Valid(score) => score,
                            ScalarFeedback::Missing => {
                                return Err(feedback_exit(
                                    self,
                                    &mut ctx,
                                    accepted,
                                    rejected,
                                    total_samples,
                                    FeedbackExitKind::Missing,
                                ));
                            }
                            ScalarFeedback::Invalid => {
                                return Err(feedback_exit(
                                    self,
                                    &mut ctx,
                                    accepted,
                                    rejected,
                                    total_samples,
                                    FeedbackExitKind::Invalid,
                                ));
                            }
                        },
                        FeedbackState::Disabled => {
                            unreachable!("run_targeted enables targeted mode before each case")
                        }
                    };
                    // The carried choice sequence (recorded or
                    // exploratory) becomes the next mutation seed.
                    // `run_targeted` always constructs recording or
                    // exploring contexts, so a sequence is always
                    // recoverable.
                    let sequence = ctx
                        .take_sequence()
                        .expect("run_targeted contexts are always recording or exploring");
                    search.corpus.admit(sequence, score);
                    accepted += 1;
                    continue;
                }
                Ok(Err(err)) => format!("{err}"),
                Err(panic) => {
                    // Defensive: an IterationRejected marker without a
                    // stored rejection state shouldn't happen because
                    // `reject_case` (and the exploratory draw cap)
                    // always set the state before resuming unwind. Keep
                    // the same guard as `run` so both entry points
                    // treat a stray marker identically.
                    if is_iteration_rejected(&*panic) {
                        rejected += 1;
                        if rejected > rejection_cap {
                            return Err(too_many_rejections(
                                self,
                                &mut ctx,
                                accepted,
                                rejected,
                                total_samples,
                                std::panic::Location::caller(),
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
                total_samples,
            };
            let generated = ctx.take_generated();
            return Err(Error::from_panic(
                self.seed,
                accepted,
                self.iterations,
                message,
                generated,
                self.stats,
                true,
            ));
        }
        self.stats = Stats {
            accepted_iterations: accepted,
            rejected_iterations: rejected,
            total_samples,
        };
        Ok(())
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
                        self.iterations,
                        rejected,
                        state.location,
                        generated,
                        self.stats,
                        false,
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
                                self.iterations,
                                rejected,
                                unknown_location,
                                generated,
                                self.stats,
                                false,
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
                self.iterations,
                message,
                generated,
                self.stats,
                false,
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

/// Which feedback validation failed, for [`feedback_exit`].
#[derive(Debug, Clone, Copy)]
enum FeedbackExitKind {
    Missing,
    Invalid,
}

/// Record run stats and build the missing / invalid feedback exit error.
fn feedback_exit(
    runner: &mut Runner,
    ctx: &mut TestCaseContext,
    accepted: usize,
    rejected: usize,
    total_samples: usize,
    kind: FeedbackExitKind,
) -> Error {
    runner.stats = Stats {
        accepted_iterations: accepted,
        rejected_iterations: rejected,
        total_samples,
    };
    let generated = ctx.take_generated();
    match kind {
        FeedbackExitKind::Missing => Error::from_missing_feedback(
            runner.seed,
            accepted,
            runner.iterations,
            generated,
            runner.stats,
        ),
        FeedbackExitKind::Invalid => Error::from_invalid_feedback(
            runner.seed,
            accepted,
            runner.iterations,
            generated,
            runner.stats,
        ),
    }
}

/// Record run stats and build the targeted too-many-rejections exit
/// error.
fn too_many_rejections(
    runner: &mut Runner,
    ctx: &mut TestCaseContext,
    accepted: usize,
    rejected: usize,
    total_samples: usize,
    location: &'static std::panic::Location<'static>,
) -> Error {
    runner.stats = Stats {
        accepted_iterations: accepted,
        rejected_iterations: rejected,
        total_samples,
    };
    let generated = ctx.take_generated();
    Error::from_too_many_rejections(
        runner.seed,
        accepted,
        runner.iterations,
        rejected,
        location,
        generated,
        runner.stats,
        true,
    )
}

/// One candidate in the targeted corpus: the recorded choice sequence
/// of an accepted case and its scalar score.
///
/// For exploratory candidates the sequence's spans are a lineage log of
/// the execution that produced it, not a faithful record of a single
/// deterministic run — mutation may change the control flow between
/// generations. Current consumers (mutation) ignore spans entirely.
struct CorpusEntry {
    sequence: ChoiceSequence,
    score: f64,
}

/// Bounded top-k corpus of accepted targeted cases.
///
/// Admission: while the corpus has fewer than `CORPUS_SIZE` entries,
/// every accepted case is kept; once full, an entry is replaced only
/// when the new score beats the lowest-scored entry. Ties keep the
/// incumbent (first arrival wins). Eviction is therefore fully
/// deterministic for a fixed candidate stream.
struct Corpus {
    entries: Vec<CorpusEntry>,
}

impl Corpus {
    fn new() -> Self {
        Self {
            entries: Vec::with_capacity(CORPUS_SIZE),
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Index of the lowest-scored entry, first among ties.
    fn min_index(&self) -> usize {
        let mut min_idx = 0;
        for (i, entry) in self.entries.iter().enumerate().skip(1) {
            if entry.score < self.entries[min_idx].score {
                min_idx = i;
            }
        }
        min_idx
    }

    fn admit(&mut self, sequence: ChoiceSequence, score: f64) -> bool {
        if self.entries.len() < CORPUS_SIZE {
            self.entries.push(CorpusEntry { sequence, score });
            return true;
        }
        // Replace the lowest-scored entry (first among ties) if the
        // new score beats it.
        let min_idx = self.min_index();
        if score > self.entries[min_idx].score {
            self.entries[min_idx] = CorpusEntry { sequence, score };
            return true;
        }
        false
    }

    fn lowest(&self) -> &CorpusEntry {
        &self.entries[self.min_index()]
    }
}

/// Per-run search state owned by the runner: the corpus and the search
/// PRNG from which every exploratory decision (entry pick, mutation
/// site, restart roll, exploratory draw seed) is derived in fixed
/// order.
struct TargetedSearch {
    corpus: Corpus,
    prng: XoshiroState,
}

impl TargetedSearch {
    fn new(seed: u64) -> Self {
        Self {
            corpus: Corpus::new(),
            prng: XoshiroState::from_seed(seed),
        }
    }
    /// Build the context for the next candidate.
    ///
    /// - While the corpus is empty, or with probability
    ///   `1 / RANDOM_RESTART_DENOM`, a fresh candidate is generated
    ///   from a case seed derived from the search PRNG (recording mode).
    /// - Otherwise a corpus entry is picked (with probability
    ///   `1 / LOW_SCORE_DENOM` the lowest-scored one), its draws are
    ///   mutated within their recorded constraints, and the result is
    ///   explored (replay + generated tail draws).
    ///
    /// The search PRNG is consumed per candidate in this fixed order:
    /// the restart roll first (skipped while the corpus is empty),
    /// then — for exploratory candidates — the explore seed, the
    /// corpus pick (a low-score roll and, when it misses, an index
    /// roll), and finally the mutation rolls. This fixed order keeps
    /// the run reproducible from the seed.
    fn next_context(&mut self) -> TestCaseContext {
        let restart = self.corpus.is_empty() || self.prng.sample_below(RANDOM_RESTART_DENOM) == 0;
        if restart {
            let case_seed = self.prng.next_u64();
            TestCaseContext::recording(case_seed)
        } else {
            let explore_seed = self.prng.next_u64();
            let mut sequence = self.pick().sequence.clone();
            mutate_sequence(&mut sequence, &mut self.prng);
            TestCaseContext::exploring(sequence, explore_seed)
        }
    }

    /// Pick a corpus entry: usually uniform, with probability
    /// `1 / LOW_SCORE_DENOM` the lowest-scored entry.
    fn pick(&mut self) -> &CorpusEntry {
        if self.prng.sample_below(LOW_SCORE_DENOM) == 0 {
            self.corpus.lowest()
        } else {
            let idx = self.prng.sample_below(self.corpus.entries.len() as u64) as usize;
            &self.corpus.entries[idx]
        }
    }
}

/// Rewrite a candidate's draws for the next generation.
///
/// Each draw is rewritten with probability `1 / MUTATION_DENOM`:
/// bounded-domain draws (Bounded / Choice) get a fresh value inside
/// their recorded constraint, while constraint-free draws (Raw: raw
/// bytes, string payload, …) are regenerated as a whole — never
/// byte-mutated in place. The draw count and the recorded attempt-span
/// structure are preserved, so the mutated sequence still replays
/// structurally.
fn mutate_sequence(sequence: &mut ChoiceSequence, prng: &mut XoshiroState) {
    let (draws, metas) = sequence.draws_and_metas();
    for (draw, meta) in draws.iter_mut().zip(metas.iter()) {
        if prng.sample_below(MUTATION_DENOM) != 0 {
            continue;
        }
        match meta {
            ChoiceMeta::Bounded { bound } => {
                if draw.len() != 8 {
                    // Bounded/Choice draws are exactly eight bytes (the
                    // rejection-sampling core draws a u64); anything
                    // else cannot be rewritten in place.
                    continue;
                }
                // bound <= 1 never occurs (sample_below returns early
                // for n == 1 without drawing), kept as a defensive
                // guard against a future primitive that records a
                // singleton domain.
                if *bound <= 1 {
                    continue;
                }
                let new_value = prng.sample_below(*bound);
                draw[..8].copy_from_slice(&new_value.to_le_bytes());
            }
            ChoiceMeta::Choice { len } => {
                if draw.len() != 8 {
                    continue;
                }
                if *len <= 1 {
                    continue;
                }
                let new_value = prng.sample_below(*len as u64);
                draw[..8].copy_from_slice(&new_value.to_le_bytes());
            }
            ChoiceMeta::Integer => {
                // Plain integer draw: rewrite to any value of the same
                // width (the full domain is valid).
                match draw.len() {
                    1 => draw.copy_from_slice(&[prng.next_u64() as u8]),
                    2 => draw.copy_from_slice(&(prng.next_u64() as u16).to_le_bytes()),
                    4 => draw.copy_from_slice(&(prng.next_u64() as u32).to_le_bytes()),
                    8 => draw.copy_from_slice(&prng.next_u64().to_le_bytes()),
                    16 => {
                        let lo = prng.next_u64();
                        let hi = prng.next_u64();
                        draw[..8].copy_from_slice(&lo.to_le_bytes());
                        draw[8..].copy_from_slice(&hi.to_le_bytes());
                    }
                    width => {
                        // Unreachable: Integer draws come from the
                        // sample_u*/sample_i* primitives whose widths
                        // are 1, 2, 4, 8, or 16 bytes. Fall back to a
                        // full regeneration if a future primitive
                        // records a different width.
                        debug_assert!(false, "unexpected integer draw width: {width}");
                        prng.fill(draw);
                    }
                }
            }
            ChoiceMeta::Raw => {
                // Constraint-free draw: regenerate the whole draw
                // rather than byte-mutating it.
                prng.fill(draw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Corpus admission ===

    #[test]
    fn corpus_admits_everything_until_full() {
        let mut corpus = Corpus::new();
        for i in 0..CORPUS_SIZE {
            assert!(
                corpus.admit(ChoiceSequence::default(), i as f64),
                "entry {i} must be admitted while below capacity"
            );
        }
        assert_eq!(corpus.entries.len(), CORPUS_SIZE);
    }

    #[test]
    fn corpus_replaces_lowest_score_when_full() {
        let mut corpus = Corpus::new();
        for i in 0..CORPUS_SIZE {
            corpus.admit(ChoiceSequence::default(), i as f64);
        }
        assert!(
            corpus.admit(ChoiceSequence::default(), CORPUS_SIZE as f64),
            "a new best score must be admitted"
        );
        let scores: Vec<f64> = corpus.entries.iter().map(|e| e.score).collect();
        assert!(scores.contains(&(CORPUS_SIZE as f64)));
        assert!(
            !scores.contains(&0.0),
            "the lowest score must have been evicted: {scores:?}"
        );
    }

    #[test]
    fn corpus_keeps_incumbent_on_tie_when_full() {
        let mut corpus = Corpus::new();
        for i in 0..CORPUS_SIZE {
            corpus.admit(ChoiceSequence::default(), i as f64);
        }
        // A new score tied with the lowest-scored entry must not
        // replace the incumbent (first arrival wins).
        assert!(!corpus.admit(ChoiceSequence::default(), 0.0));
        let scores: Vec<f64> = corpus.entries.iter().map(|e| e.score).collect();
        assert!(scores.contains(&0.0), "incumbent must survive: {scores:?}");
    }

    #[test]
    fn corpus_rejects_score_not_beating_lowest() {
        let mut corpus = Corpus::new();
        for i in 0..CORPUS_SIZE {
            corpus.admit(ChoiceSequence::default(), i as f64);
        }
        assert!(
            !corpus.admit(ChoiceSequence::default(), -1.0),
            "a score below the lowest must be rejected"
        );
        assert_eq!(corpus.entries.len(), CORPUS_SIZE);
    }

    // === mutate_sequence ===

    #[test]
    fn mutation_stays_within_bounded_domain() {
        let mut prng = XoshiroState::from_seed(1234);
        let mut mutated = false;
        let mut raw_regenerated = false;
        let original_raw = vec![0xAB; 8];
        for _ in 0..100 {
            let mut seq = ChoiceSequence::default();
            seq.push_draw(
                7u64.to_le_bytes().to_vec(),
                ChoiceMeta::Bounded { bound: 10 },
            );
            // Raw draw: regenerated as a whole when mutated (never
            // byte-mutated in place).
            seq.push_draw(original_raw.clone(), ChoiceMeta::Raw);
            mutate_sequence(&mut seq, &mut prng);
            let x = u64::from_le_bytes(seq.draws()[0][..8].try_into().unwrap());
            assert!(x < 10, "mutated bounded draw {x} escaped its domain");
            if x != 7 {
                mutated = true;
            }
            if seq.draws()[1] != original_raw {
                raw_regenerated = true;
            }
        }
        assert!(mutated, "mutation must have occurred at least once");
        assert!(
            raw_regenerated,
            "raw draw regeneration must have occurred at least once"
        );
    }

    #[test]
    fn mutation_preserves_choice_index_domain() {
        let mut prng = XoshiroState::from_seed(99);
        for _ in 0..100 {
            let mut seq = ChoiceSequence::default();
            seq.push_draw(3u64.to_le_bytes().to_vec(), ChoiceMeta::Choice { len: 4 });
            mutate_sequence(&mut seq, &mut prng);
            let idx = u64::from_le_bytes(seq.draws()[0][..8].try_into().unwrap());
            assert!(idx < 4, "mutated choice index {idx} escaped its domain");
        }
    }
}

// === TargetedSearch wiring ===

#[test]
fn targeted_search_evolves_corpus_entries() {
    let mut search = TargetedSearch::new(42);
    let mut seq = ChoiceSequence::default();
    seq.push_draw(
        7u64.to_le_bytes().to_vec(),
        ChoiceMeta::Bounded { bound: 10 },
    );
    search.corpus.admit(seq, 1.0);
    // With a one-eighth restart probability, most picks are
    // exploratory: they replay the admitted draw under the same
    // declared constraint (possibly mutated, always in domain).
    // Recording (fresh) candidates draw unconstrained random bytes
    // instead. Deterministic per seed.
    let mut in_domain = 0;
    for _ in 0..16 {
        let mut ctx = search.next_context();
        // Mimic the property's generator declaring the same
        // constraint for this draw position.
        ctx.set_next_choice_meta(ChoiceMeta::Bounded { bound: 10 });
        let mut buf = [0u8; 8];
        ctx.fill(&mut buf);
        let x = u64::from_le_bytes(buf);
        let _ = ctx.take_sequence();
        if x < 10 {
            in_domain += 1;
        }
    }
    assert!(
        in_domain >= 8,
        "most candidates must replay an in-domain draw: {in_domain}"
    );
}

#[test]
fn mutation_rewrites_integer_draws() {
    let mut prng = XoshiroState::from_seed(77);
    let mut changed = false;
    for _ in 0..100 {
        let mut seq = ChoiceSequence::default();
        seq.push_draw(5u64.to_le_bytes().to_vec(), ChoiceMeta::Integer);
        mutate_sequence(&mut seq, &mut prng);
        let x = u64::from_le_bytes(seq.draws()[0][..8].try_into().unwrap());
        if x != 5 {
            changed = true;
        }
    }
    assert!(changed, "integer draws must be rewritten to another value");
}

#[test]
fn mutation_rewrites_integer_draws_across_widths() {
    let mut prng = XoshiroState::from_seed(31337);
    // Every recorded width must be rewritable to a new value.
    let mut changed = 0u32;
    for width in [1usize, 2, 4, 8, 16] {
        for _ in 0..64 {
            let mut seq = ChoiceSequence::default();
            seq.push_draw(vec![0xAB; width], ChoiceMeta::Integer);
            let before = seq.draws()[0].clone();
            mutate_sequence(&mut seq, &mut prng);
            if seq.draws()[0] != before {
                changed += 1;
                break;
            }
        }
    }
    assert_eq!(changed, 5, "every integer width must be rewritable");
}

#[test]
fn mutation_rewrites_upper_bytes_of_16_byte_integer_draw() {
    // Width 16 is written as two 8-byte halves; a mutation bug that
    // only rewrote the low half would leave high values untouched.
    let mut prng = XoshiroState::from_seed(1337);
    let mut low_changed = false;
    let mut high_changed = false;
    for _ in 0..256 {
        let mut seq = ChoiceSequence::default();
        seq.push_draw(vec![0xAB; 16], ChoiceMeta::Integer);
        let before = seq.draws()[0].clone();
        mutate_sequence(&mut seq, &mut prng);
        let after = &seq.draws()[0];
        low_changed |= after[..8] != before[..8];
        high_changed |= after[8..] != before[8..];
        if low_changed && high_changed {
            break;
        }
    }
    assert!(low_changed, "low 8 bytes must be rewritable");
    assert!(high_changed, "high 8 bytes must be rewritable");
}
