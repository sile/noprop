//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::rng::{
    ChoiceMeta, ChoiceSequence, FeedbackState, ScalarFeedback, XoshiroState, is_iteration_rejected,
    is_replay_abort,
};
use crate::{Error, Result, TestCaseContext};

/// Maximum number of candidates kept in the targeted corpus (top-k).
const CORPUS_SIZE: usize = 64;

/// Denominator of the per-draw mutation probability: one in
/// `MUTATION_DENOM` draws of a selected candidate are rewritten.
const MUTATION_DENOM: u64 = 4;

/// Denominator of the random-restart probability: one in
/// `RANDOM_RESTART_DENOM` candidates are freshly generated instead of
/// mutated from the corpus.
const RANDOM_RESTART_DENOM: u64 = 8;

/// Denominator of the low-score selection probability: one in
/// `LOW_SCORE_DENOM` corpus picks target the lowest-scored entry to
/// keep an alternative search path alive.
const LOW_SCORE_DENOM: u64 = 4;

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
            let mut ctx = search.next_context();
            ctx.set_inside_runner();
            ctx.enable_targeted();
            ctx.clear_generated();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut ctx)));
            let rejection = ctx.take_rejection();
            total_samples += ctx.total_samples();

            if let Some(state) = rejection {
                let _ = outcome;
                rejected += 1;
                if rejected > rejection_cap {
                    self.stats = Stats {
                        accepted_iterations: accepted,
                        rejected_iterations: rejected,
                        total_samples,
                    };
                    let generated = ctx.take_generated();
                    return Err(Error::from_too_many_rejections_targeted(
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
                    // Accepted case: feedback is mandatory in targeted mode.
                    let feedback = ctx.take_feedback();
                    let score = match feedback {
                        FeedbackState::Targeted { max_score } => match max_score {
                            ScalarFeedback::Valid(score) => score,
                            ScalarFeedback::Missing => {
                                self.stats = Stats {
                                    accepted_iterations: accepted,
                                    rejected_iterations: rejected,
                                    total_samples,
                                };
                                let generated = ctx.take_generated();
                                return Err(Error::from_missing_feedback(
                                    self.seed, accepted, generated, self.stats,
                                ));
                            }
                            ScalarFeedback::Invalid => {
                                self.stats = Stats {
                                    accepted_iterations: accepted,
                                    rejected_iterations: rejected,
                                    total_samples,
                                };
                                let generated = ctx.take_generated();
                                return Err(Error::from_invalid_feedback(
                                    self.seed, accepted, generated, self.stats,
                                ));
                            }
                        },
                        FeedbackState::Disabled => {
                            unreachable!("run_targeted enables targeted mode before each case")
                        }
                    };
                    if let Some(sequence) = ctx.take_sequence() {
                        search.corpus.admit(sequence, score);
                    }
                    accepted += 1;
                    continue;
                }
                Ok(Err(err)) => format!("{err}"),
                Err(panic) => {
                    if is_iteration_rejected(&*panic) {
                        rejected += 1;
                        if rejected > rejection_cap {
                            self.stats = Stats {
                                accepted_iterations: accepted,
                                rejected_iterations: rejected,
                                total_samples,
                            };
                            let generated = ctx.take_generated();
                            let unknown_location = std::panic::Location::caller();
                            return Err(Error::from_too_many_rejections_targeted(
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
                    if is_replay_abort(&*panic) {
                        // A mutated candidate whose control flow diverged
                        // from the recorded structure is not a property
                        // failure: discard it and move on, counting it
                        // against the rejection budget so the run stays
                        // finite.
                        rejected += 1;
                        if rejected > rejection_cap {
                            self.stats = Stats {
                                accepted_iterations: accepted,
                                rejected_iterations: rejected,
                                total_samples,
                            };
                            let generated = ctx.take_generated();
                            let unknown_location = std::panic::Location::caller();
                            return Err(Error::from_too_many_rejections_targeted(
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
                total_samples,
            };
            let generated = ctx.take_generated();
            return Err(Error::from_panic_targeted(
                self.seed, accepted, message, generated, self.stats,
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
                self.seed, accepted, message, generated, self.stats,
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

/// One candidate in the targeted corpus: the recorded choice sequence
/// of an accepted case and its scalar score.
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

    fn admit(&mut self, sequence: ChoiceSequence, score: f64) -> bool {
        if self.entries.len() < CORPUS_SIZE {
            self.entries.push(CorpusEntry { sequence, score });
            return true;
        }
        // Replace the lowest-scored entry (first among ties) if the
        // new score beats it.
        let mut min_idx = 0;
        for (i, entry) in self.entries.iter().enumerate().skip(1) {
            if entry.score < self.entries[min_idx].score {
                min_idx = i;
            }
        }
        if score > self.entries[min_idx].score {
            self.entries[min_idx] = CorpusEntry { sequence, score };
            return true;
        }
        false
    }

    fn lowest(&self) -> &CorpusEntry {
        let mut min_idx = 0;
        for (i, entry) in self.entries.iter().enumerate().skip(1) {
            if entry.score < self.entries[min_idx].score {
                min_idx = i;
            }
        }
        &self.entries[min_idx]
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

/// Rewrite a candidate's draws within their recorded constraints.
///
/// Each non-`Raw` draw is rewritten with probability
/// `1 / MUTATION_DENOM` to a fresh value inside its bounded domain;
/// `Raw` draws (bytes, string payload, …) are never byte-mutated. The
/// draw count and the nested attempt-span structure are preserved, so
/// the mutated sequence still replays structurally.
fn mutate_sequence(sequence: &mut ChoiceSequence, prng: &mut XoshiroState) {
    let metas: Vec<ChoiceMeta> = sequence.metas().to_vec();
    for (draw, meta) in sequence.draws_mut().iter_mut().zip(metas.iter()) {
        if prng.sample_below(MUTATION_DENOM) != 0 {
            continue;
        }
        let bound = match meta {
            ChoiceMeta::Bounded { bound } => *bound,
            ChoiceMeta::Choice { len } => *len as u64,
            ChoiceMeta::Raw => continue,
        };
        if draw.len() < 8 || bound <= 1 {
            continue;
        }
        let new_value = prng.sample_below(bound);
        draw[..8].copy_from_slice(&new_value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::ChoiceMeta;

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
    fn corpus_keeps_incumbent_on_tie() {
        let mut corpus = Corpus::new();
        corpus.admit(ChoiceSequence::default(), 1.0);
        corpus.admit(ChoiceSequence::default(), 1.0);
        assert_eq!(corpus.entries.len(), 2, "tie must keep the incumbent");
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
        for _ in 0..100 {
            let mut seq = ChoiceSequence::default();
            seq.push_draw(
                7u64.to_le_bytes().to_vec(),
                ChoiceMeta::Bounded { bound: 10 },
            );
            seq.push_draw(vec![0xAB; 8], ChoiceMeta::Raw);
            mutate_sequence(&mut seq, &mut prng);
            let x = u64::from_le_bytes(seq.draws()[0][..8].try_into().unwrap());
            assert!(x < 10, "mutated bounded draw {x} escaped its domain");
            assert_eq!(
                seq.draws()[1],
                vec![0xAB; 8],
                "raw draw must not be mutated"
            );
        }
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
