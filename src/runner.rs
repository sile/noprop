//! Property-based test runner.

use std::panic::AssertUnwindSafe;

use crate::error::SearchPolicy;
use crate::rng::{
    ChoiceMeta, ChoiceSequence, Feature, FeedbackState, RejectionState, XoshiroState,
    is_iteration_rejected,
};
use crate::{RunError, RunResult, TestCaseContext, TestResult};

/// Maximum number of candidates kept in the semantic corpus.
///
/// All four search constants (`CORPUS_SIZE`, `MUTATION_DENOM`,
/// `RANDOM_RESTART_DENOM`, `REJECTED_PICK_DENOM`) are initial guesses;
/// their tuning is deferred until benchmark data exists.
const CORPUS_SIZE: usize = 64;

/// Denominator of the per-draw mutation probability: one in
/// `MUTATION_DENOM` draws of a selected candidate are rewritten.
const MUTATION_DENOM: u64 = 4;

/// Denominator of the random-restart probability: one in
/// `RANDOM_RESTART_DENOM` candidates are freshly generated instead of
/// mutated from the corpus.
const RANDOM_RESTART_DENOM: u64 = 8;

/// Denominator of the rejected-entry pick probability: one in
/// `REJECTED_PICK_DENOM` corpus picks target the rejected queue, which
/// is kept with lower energy than accepted entries (a scaffolding
/// toward sparse preconditions, not the main search path).
const REJECTED_PICK_DENOM: u64 = 8;

/// Maximum number of semantic features observed across a whole
/// feedback-guided run. After the cap is reached, new features are not
/// registered and never make a case interesting, so a high-cardinality
/// property cannot grow the registry without bound.
const MAX_GLOBAL_FEATURES: usize = 1024;

/// Observability data from a [`Runner::run`](Runner::run) or
/// [`Runner::run_feedback_guided`](Runner::run_feedback_guided)
/// invocation.
///
/// Read from a [`Runner`] after the run returns via
/// [`Runner::stats`](Runner::stats), and also embedded in
/// [`RunError`](crate::RunError) on
/// failure so the caller can see how far the run progressed before it
/// failed. All counters are cumulative over the whole run (across
/// every case, accepted or rejected). The corpus fields
/// ([`Stats::discovered_features`](Stats::discovered_features) and
/// [`Stats::max_corpus_size`](Stats::max_corpus_size)) are only
/// meaningful for feedback-guided runs and are 0 otherwise.
///
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
    /// origins share this single counter), and — under
    /// [`Runner::run_feedback_guided`](Runner::run_feedback_guided) — the
    /// exploratory-replay draw cap discards.
    pub rejected_cases: usize,
    /// Total number of top-level `sample_*` invocations across every
    /// case that ran. Counted per call to the primitive generator
    /// (`sample_u32`, `sample_choice`, `sample_string`, …), not per
    /// underlying byte read or dedup entry — a `sample_char` invocation
    /// that internally retries its 21-bit mask still counts as one
    /// sample. Includes samples produced by rejected cases, since
    /// those cases still consumed generator budget.
    pub total_samples: usize,
    /// Number of distinct semantic features registered in the global
    /// observation set during a feedback-guided run, capped at 1024
    /// (currently). Features registered by rejected cases are
    /// included; features of the failing case itself are not (a case
    /// is registered only after its verdict, and the failing case
    /// never reaches admission).
    pub discovered_features: usize,
    /// Combined size of the semantic corpus (accepted + rejected) at
    /// the end of a feedback-guided run. The combined size only grows
    /// and is trimmed back to the corpus cap (64) when full, so the
    /// value at the end of the run equals the maximum; the transient
    /// overshoot just before eviction is not counted (admission
    /// pushes, then trims only when over the cap).
    pub max_corpus_size: usize,
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
/// that runner-wide configuration (default rejection budget, feedback
/// mode, snapshot directory, …) can be added later without breaking
/// existing call sites. Observability data ([`Stats`]) is exposed via
/// [`Runner::stats`] after `run` returns.
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
/// decimal values as well as `0x` / `0b` / `0o` prefixed values with
/// optional `_` separators, so the hex seed printed by a failure
/// report can be pasted into the environment variable directly. The
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

/// Global rejection limit for a single [`Runner::run`](Runner::run) or
/// [`Runner::run_feedback_guided`](Runner::run_feedback_guided)
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
    /// given per run, via [`run`](Runner::run) / [`run_feedback_guided`](Runner::run_feedback_guided).
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

    /// Observability counters from the most recent
    /// [`run`](Runner::run) or
    /// [`run_feedback_guided`](Runner::run_feedback_guided)
    /// call on this runner. Returns [`Stats::default`] (all zeros)
    /// before a run has been invoked.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Feedback-guided search over semantic features reported via
    /// [`TestCaseContext::event`](crate::TestCaseContext::event) /
    /// [`bucket`](crate::TestCaseContext::bucket) /
    /// [`transition`](crate::TestCaseContext::transition).
    ///
    /// The property closure has the same shape as
    /// [`run`](Runner::run), so the same property can be exercised
    /// under both entry points. Feedback is not mandatory: an accepted
    /// case that reports no semantic feature is simply not interesting
    /// (it never enters the corpus), and no missing / invalid feedback
    /// error is raised.
    ///
    /// Candidates are produced from a bounded corpus of interesting
    /// cases: a case that registers at least one globally unobserved
    /// feature is admitted, mutated within its recorded constraints,
    /// and replayed with exploratory generation for draws the mutation
    /// introduces. Accepted and rejected cases live in
    /// separate queues (the rejected queue is picked with probability
    /// `1 / REJECTED_PICK_DENOM`); with probability
    /// `1 / RANDOM_RESTART_DENOM` a candidate is freshly generated
    /// instead, so the search can escape local optima.
    ///
    /// The rejection semantics (global rejection cap, `Stats`, and the
    /// `cases` budget counting only accepted cases) match
    /// [`run`](Runner::run).
    ///
    /// # Example
    ///
    /// ```
    /// let mut runner = noprop::Runner::new(0xDEAD_BEEF);
    /// runner
    ///     .run_feedback_guided(16, |ctx| {
    ///         let x = noprop::sample_u32(ctx);
    ///         if x == 0 {
    ///             ctx.event("zero");
    ///         }
    ///         Ok(())
    ///     })
    ///     .expect("feedback-guided run must succeed");
    /// ```
    pub fn run_feedback_guided<F>(&mut self, cases: usize, f: F) -> RunResult
    where
        F: Fn(&mut TestCaseContext) -> TestResult,
    {
        self.stats = Stats::default();
        let mut search = CorpusGuidedSearch::new(self.seed);
        let rejection_cap = rejection_limit(cases);
        let mut accepted: usize = 0;
        let mut rejected: usize = 0;
        let mut total_samples: usize = 0;

        while accepted < cases {
            // Each iteration gets a fresh context (recording or
            // exploratory), so there is no per-case state to clear.
            let mut ctx = search.next_context();
            ctx.set_inside_runner();
            ctx.enable_corpus_guided();
            let verdict = run_case(&f, &mut ctx);
            total_samples = total_samples.saturating_add(ctx.total_samples());
            match verdict {
                CaseVerdict::Rejected(state) => {
                    rejected += 1;
                    if rejected > rejection_cap {
                        // Cap exceeded: report before consuming the
                        // feedback, so the error can carry the last
                        // rejected case's semantic features and
                        // candidate index (drained inside
                        // `too_many_rejections`).
                        record_corpus_stats(self, &search, accepted, rejected, total_samples);
                        return Err(too_many_rejections(
                            self,
                            &mut ctx,
                            state.location,
                            SearchPolicy::FeedbackGuided,
                            cases,
                        ));
                    }
                    // A rejected case may still register novel features
                    // and enter the rejected queue as scaffolding toward
                    // sparse preconditions.
                    let feedback = ctx.take_feedback();
                    if let FeedbackState::SemanticCoverage(mut cov) = feedback {
                        let features = cov.take_features();
                        if let Some(sequence) = ctx.take_sequence() {
                            search.corpus.admit_rejected(sequence, features);
                        }
                    }
                    continue;
                }
                CaseVerdict::Completed(CaseOutcome::Passed) => {
                    // Accepted case: the reported features decide
                    // interest.
                    let feedback = ctx.take_feedback();
                    let mut cov = match feedback {
                        FeedbackState::SemanticCoverage(cov) => cov,
                        _ => unreachable!(
                            "run_feedback_guided enables feedback-guided mode before each case"
                        ),
                    };
                    let features = cov.take_features();
                    if let Some(sequence) = ctx.take_sequence() {
                        search.corpus.admit_accepted(sequence, features);
                    }
                    accepted += 1;
                    continue;
                }
                CaseVerdict::Completed(CaseOutcome::Failed(message)) => {
                    record_corpus_stats(self, &search, accepted, rejected, total_samples);
                    let generated = ctx.take_generated();
                    let feedback = ctx.take_feedback();
                    let semantic_features = match feedback {
                        FeedbackState::SemanticCoverage(mut cov) => cov.take_features(),
                        _ => unreachable!(
                            "run_feedback_guided enables feedback-guided mode before each case"
                        ),
                    };
                    return Err(RunError::from_panic(
                        self.seed,
                        cases,
                        message,
                        generated,
                        self.stats,
                        SearchPolicy::FeedbackGuided,
                    )
                    .with_semantic(
                        semantic_features,
                        // The failing case is the current attempt: it
                        // is neither accepted nor rejected, so its
                        // candidate index is one past the completed
                        // attempts.
                        accepted + rejected + 1,
                    ));
                }
            }
        }
        record_corpus_stats(self, &search, accepted, rejected, total_samples);
        Ok(())
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
                        record_stats(self, accepted, rejected, ctx.total_samples(), 0, 0);
                        let generated = ctx.take_generated();
                        return Err(RunError::from_too_many_rejections(
                            self.seed,
                            cases,
                            state.location,
                            generated,
                            self.stats,
                            SearchPolicy::Uniform,
                        ));
                    }
                    continue;
                }
                CaseVerdict::Completed(CaseOutcome::Passed) => {
                    accepted += 1;
                    continue;
                }
                CaseVerdict::Completed(CaseOutcome::Failed(message)) => {
                    record_stats(self, accepted, rejected, ctx.total_samples(), 0, 0);
                    let generated = ctx.take_generated();
                    return Err(RunError::from_panic(
                        self.seed,
                        cases,
                        message,
                        generated,
                        self.stats,
                        SearchPolicy::Uniform,
                    ));
                }
            }
        }
        record_stats(self, accepted, rejected, ctx.total_samples(), 0, 0);
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
            "noprop::Runner {{ seed: {:#018x}, stats: {{ accepted: {}, rejected: {}, total_samples: {}, discovered_features: {}, max_corpus_size: {} }} }}",
            self.seed,
            self.stats.accepted_cases,
            self.stats.rejected_cases,
            self.stats.total_samples,
            self.stats.discovered_features,
            self.stats.max_corpus_size,
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
/// Shared by the uniform and feedback-guided entry points: the
/// `catch_unwind` boundary, the stored-rejection precedence, the
/// stray-marker guard, and panic-message extraction are identical
/// across modes.
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
            // `reject_case` (and the exploratory draw cap) always set
            // the state before resuming unwind. If it somehow does,
            // treat it as rejection rather than as a property failure
            // with an opaque payload.
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
/// (success, property failure, rejection cap, feedback failure)
/// reports the same counters.
fn record_stats(
    runner: &mut Runner,
    accepted: usize,
    rejected: usize,
    total_samples: usize,
    discovered_features: usize,
    max_corpus_size: usize,
) {
    runner.stats = Stats {
        accepted_cases: accepted,
        rejected_cases: rejected,
        total_samples,
        discovered_features,
        max_corpus_size,
    };
}

/// The corpus-derived stats fields: the size of the global feature
/// observation set and the combined (accepted + rejected) corpus size
/// at the current search state.
fn corpus_stats(search: &CorpusGuidedSearch) -> (usize, usize) {
    (
        search.corpus.observed.len(),
        search.corpus.accepted.len() + search.corpus.rejected.len(),
    )
}

/// Record the run progress counters including the corpus-derived
/// fields at the current search state.
fn record_corpus_stats(
    runner: &mut Runner,
    search: &CorpusGuidedSearch,
    accepted: usize,
    rejected: usize,
    total_samples: usize,
) {
    let (discovered_features, max_corpus_size) = corpus_stats(search);
    record_stats(
        runner,
        accepted,
        rejected,
        total_samples,
        discovered_features,
        max_corpus_size,
    );
}

/// Build the too-many-rejections exit error. The caller must have
/// recorded the run progress counters first (via `record_stats` or
/// `record_corpus_stats`); this helper reads them back from
/// `runner.stats`.
///
/// For feedback-guided runs, the error additionally carries the semantic
/// features of the last rejected case and its candidate index (the
/// ordinal of the attempt that exceeded the cap, derived from the
/// recorded stats: `accepted + rejected`).
fn too_many_rejections(
    runner: &mut Runner,
    ctx: &mut TestCaseContext,
    location: &'static std::panic::Location<'static>,
    policy: SearchPolicy,
    cases: usize,
) -> RunError {
    let generated = ctx.take_generated();
    let err = RunError::from_too_many_rejections(
        runner.seed,
        cases,
        location,
        generated,
        runner.stats,
        policy,
    );
    if matches!(policy, SearchPolicy::FeedbackGuided) {
        let features = match ctx.take_feedback() {
            FeedbackState::SemanticCoverage(mut cov) => cov.take_features(),
            _ => Vec::new(),
        };
        let last_candidate = runner.stats.accepted_cases + runner.stats.rejected_cases;
        return err.with_semantic(features, last_candidate);
    }
    err
}

/// One candidate in the semantic corpus: the recorded choice sequence
/// of an interesting case and the features it newly registered.
///
/// The sequence carries whatever the accepted case recorded: recorded
/// cases keep their attempt spans, exploratory cases have none
/// (exploration records no spans). Mutation reads only the draws and
/// their metadata, never the spans.
struct SemanticEntry {
    sequence: ChoiceSequence,
    /// The features this case newly registered in the global
    /// observation set. The count is the eviction criterion (fewest
    /// novel features evicted first).
    novel: Vec<Feature>,
}

/// Bounded corpus of interesting cases for feedback-guided search.
///
/// Accepted and rejected cases live in separate queues; the combined
/// size is capped at `CORPUS_SIZE`. Admission and eviction are
/// deterministic:
///
/// - A case that registers at least one novel feature is admitted
///   while the combined size is below the cap.
/// - Once full, the entry with the fewest newly registered features is
///   evicted; ties evict the earliest arrival.
/// - A case with no novel feature is never admitted.
struct SemanticCorpus {
    accepted: Vec<SemanticEntry>,
    rejected: Vec<SemanticEntry>,
    /// Globally observed features in first-registration order, capped
    /// at `MAX_GLOBAL_FEATURES`. A feature already present here is not
    /// interesting again.
    observed: Vec<Feature>,
}

impl SemanticCorpus {
    fn new() -> Self {
        Self {
            accepted: Vec::with_capacity(CORPUS_SIZE),
            rejected: Vec::with_capacity(CORPUS_SIZE),
            observed: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.accepted.is_empty() && self.rejected.is_empty()
    }

    /// The features of `case_features` that are not yet in the global
    /// observation set, in report order.
    fn novel_features(&self, case_features: &[Feature]) -> Vec<Feature> {
        case_features
            .iter()
            .filter(|f| !self.observed.contains(f))
            .cloned()
            .collect()
    }

    /// Register novel features into the global observation set,
    /// stopping at `MAX_GLOBAL_FEATURES`. Returns the features that
    /// were actually registered (an empty result means the case added
    /// nothing new and is not interesting).
    ///
    /// `novel` must be the output of [`novel_features`](Self::novel_features)
    /// (or otherwise contain only features absent from `observed`);
    /// duplicates are not checked here.
    fn register(&mut self, novel: &[Feature]) -> Vec<Feature> {
        let mut registered = Vec::new();
        for feature in novel {
            if self.observed.len() >= MAX_GLOBAL_FEATURES {
                break;
            }
            self.observed.push(feature.clone());
            registered.push(feature.clone());
        }
        registered
    }

    /// Admit an accepted case. Returns `true` when the case entered
    /// the corpus.
    fn admit_accepted(&mut self, sequence: ChoiceSequence, case_features: Vec<Feature>) -> bool {
        let novel = self.novel_features(&case_features);
        let registered = self.register(&novel);
        if !registered.is_empty() {
            self.accepted.push(SemanticEntry {
                sequence,
                novel: registered,
            });
            self.evict_if_over_capacity();
            return true;
        }
        false
    }

    /// Admit a rejected case that registered novel features, into the
    /// rejected queue (kept as low-energy scaffolding).
    fn admit_rejected(&mut self, sequence: ChoiceSequence, case_features: Vec<Feature>) -> bool {
        let novel = self.novel_features(&case_features);
        let registered = self.register(&novel);
        if registered.is_empty() {
            return false;
        }
        self.rejected.push(SemanticEntry {
            sequence,
            novel: registered,
        });
        self.evict_if_over_capacity();
        true
    }

    /// Index of the weakest entry across both queues: fewest newly
    /// registered features first; ties keep the earlier arrival.
    fn weakest_overall(&self) -> usize {
        let mut weakest: Option<(&SemanticEntry, usize)> = None;
        for (i, entry) in self.accepted.iter().chain(self.rejected.iter()).enumerate() {
            match weakest {
                None => weakest = Some((entry, i)),
                Some((incumbent, _)) => {
                    if entry.novel.len() < incumbent.novel.len() {
                        weakest = Some((entry, i));
                    }
                }
            }
        }
        weakest.expect("corpus is non-empty while evicting").1
    }

    /// Evict the weakest entry (across both queues) while the combined
    /// size exceeds `CORPUS_SIZE`.
    fn evict_if_over_capacity(&mut self) {
        while self.accepted.len() + self.rejected.len() > CORPUS_SIZE {
            let idx = self.weakest_overall();
            if idx < self.accepted.len() {
                self.accepted.remove(idx);
            } else {
                self.rejected.remove(idx - self.accepted.len());
            }
        }
    }
}

/// Per-run search state owned by the runner: the semantic corpus and
/// the search PRNG from which every exploratory decision (entry pick,
/// mutation site, restart roll, exploratory draw seed) is derived in
/// fixed order.
struct CorpusGuidedSearch {
    corpus: SemanticCorpus,
    prng: XoshiroState,
}

impl CorpusGuidedSearch {
    fn new(seed: u64) -> Self {
        Self {
            corpus: SemanticCorpus::new(),
            prng: XoshiroState::from_seed(seed),
        }
    }

    /// Build the context for the next candidate.
    ///
    /// - While the corpus is empty, or with probability
    ///   `1 / RANDOM_RESTART_DENOM`, a fresh candidate is generated
    ///   from a case seed derived from the search PRNG (recording
    ///   mode).
    /// - Otherwise, with probability `1 / REJECTED_PICK_DENOM` the
    ///   rejected queue is picked (when non-empty); otherwise an
    ///   accepted entry is picked uniformly — its draws are mutated
    ///   within their recorded constraints, and the result is explored
    ///   (replay + generated tail draws).
    ///
    /// The search PRNG is consumed per candidate in this fixed order:
    /// the restart roll first (skipped while the corpus is empty),
    /// then — for exploratory candidates — the explore seed, the
    /// rejected-queue roll (skipped while it is empty), the index roll,
    /// and finally the mutation rolls. This fixed order keeps the run
    /// reproducible from the seed.
    fn next_context(&mut self) -> TestCaseContext {
        let restart = self.corpus.is_empty() || self.prng.sample_below(RANDOM_RESTART_DENOM) == 0;
        if restart {
            let case_seed = self.prng.next_u64();
            TestCaseContext::recording(case_seed)
        } else {
            let explore_seed = self.prng.next_u64();
            // Prefer the accepted queue, except with probability
            // `1 / REJECTED_PICK_DENOM` pick the rejected queue when it
            // is non-empty. The rejected queue is the only source while
            // the accepted queue is empty (an early rejected case that
            // registered novel features).
            let use_rejected = !self.corpus.rejected.is_empty()
                && (self.corpus.accepted.is_empty()
                    || self.prng.sample_below(REJECTED_PICK_DENOM) == 0);
            let mut sequence = if use_rejected {
                let idx = self.prng.sample_below(self.corpus.rejected.len() as u64) as usize;
                self.corpus.rejected[idx].sequence.clone()
            } else {
                let idx = self.prng.sample_below(self.corpus.accepted.len() as u64) as usize;
                self.corpus.accepted[idx].sequence.clone()
            };
            mutate_sequence(&mut sequence, &mut self.prng);
            TestCaseContext::exploring(sequence, explore_seed)
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

    // === SemanticCorpus admission / eviction ===

    fn event_feature(label: &'static str) -> Feature {
        Feature {
            label,
            kind: crate::rng::FeatureKind::Event(crate::rng::EventBucket::One),
        }
    }

    fn bucket_feature(label: &'static str, value: u64) -> Feature {
        Feature {
            label,
            kind: crate::rng::FeatureKind::Bucket { value },
        }
    }

    #[test]
    fn semantic_corpus_admits_novel_features() {
        let mut corpus = SemanticCorpus::new();
        assert!(
            corpus.admit_accepted(ChoiceSequence::default(), vec![event_feature("a")]),
            "a case with a novel feature must be admitted"
        );
        assert!(
            !corpus.admit_accepted(ChoiceSequence::default(), vec![event_feature("a")]),
            "a case with only known features must not be admitted"
        );
        assert_eq!(corpus.accepted.len(), 1);
    }

    #[test]
    fn semantic_corpus_register_stops_at_global_cap() {
        // The global feature registry must stop growing at
        // MAX_GLOBAL_FEATURES: a novel feature reported after the cap
        // never makes a case interesting, so a high-cardinality
        // property cannot grow memory without bound.
        let mut corpus = SemanticCorpus::new();
        for i in 0..MAX_GLOBAL_FEATURES {
            assert!(corpus.admit_accepted(
                ChoiceSequence::default(),
                vec![bucket_feature("f", i as u64)],
            ));
        }
        assert_eq!(corpus.observed.len(), MAX_GLOBAL_FEATURES);
        assert!(
            !corpus.admit_accepted(
                ChoiceSequence::default(),
                vec![bucket_feature("f", MAX_GLOBAL_FEATURES as u64)],
            ),
            "a novel feature after the cap must not be admitted"
        );
        assert_eq!(corpus.observed.len(), MAX_GLOBAL_FEATURES);
        assert_eq!(
            corpus.accepted.len(),
            CORPUS_SIZE,
            "the corpus itself stays bounded by eviction"
        );
    }

    #[test]
    fn semantic_corpus_register_partially_registers_at_cap_boundary() {
        // At the cap boundary, only the features that fit are
        // registered: the case is admitted with the truncated novel
        // set, and the overflow feature is not registered.
        let mut corpus = SemanticCorpus::new();
        let prefix: Vec<Feature> = (0..MAX_GLOBAL_FEATURES - 1)
            .map(|i| bucket_feature("g", i as u64))
            .collect();
        assert_eq!(corpus.register(&prefix).len(), MAX_GLOBAL_FEATURES - 1);
        let fits = bucket_feature("g", (MAX_GLOBAL_FEATURES - 1) as u64);
        let overflows = bucket_feature("g", MAX_GLOBAL_FEATURES as u64);
        assert!(corpus.admit_accepted(ChoiceSequence::default(), vec![fits.clone(), overflows],));
        assert_eq!(corpus.accepted[0].novel, vec![fits]);
        assert_eq!(corpus.observed.len(), MAX_GLOBAL_FEATURES);
    }

    #[test]
    fn semantic_corpus_rejected_queue_holds_novel_cases() {
        let mut corpus = SemanticCorpus::new();
        assert!(
            corpus.admit_rejected(ChoiceSequence::default(), vec![event_feature("r")]),
            "a rejected case with a novel feature must enter the rejected queue"
        );
        assert!(
            !corpus.admit_rejected(ChoiceSequence::default(), vec![event_feature("r")]),
            "a rejected case with only known features must not be kept"
        );
        assert_eq!(corpus.rejected.len(), 1);
        assert_eq!(corpus.observed.len(), 1);
    }

    #[test]
    fn semantic_corpus_evicts_fewest_features_first() {
        let mut corpus = SemanticCorpus::new();
        // Fill the corpus with distinct single-feature entries, then a
        // two-feature entry on top.
        for i in 0..CORPUS_SIZE {
            corpus.admit_accepted(
                ChoiceSequence::default(),
                vec![bucket_feature("single", i as u64)],
            );
        }
        assert_eq!(corpus.accepted.len(), CORPUS_SIZE);
        corpus.admit_accepted(
            ChoiceSequence::default(),
            vec![event_feature("two-a"), event_feature("two-b")],
        );
        assert_eq!(corpus.accepted.len(), CORPUS_SIZE);
        // The two-feature entry survives; a single-feature entry was
        // evicted (the earliest arrival).
        assert!(
            corpus
                .accepted
                .iter()
                .any(|e| e.novel.contains(&event_feature("two-a"))),
            "the feature-richest entry must survive eviction"
        );
        assert!(
            !corpus
                .accepted
                .iter()
                .any(|e| e.novel.contains(&bucket_feature("single", 0))),
            "the earliest single-feature entry must be evicted"
        );
    }

    #[test]
    fn semantic_corpus_evicts_across_both_queues() {
        let mut corpus = SemanticCorpus::new();
        for i in 0..CORPUS_SIZE {
            corpus.admit_accepted(
                ChoiceSequence::default(),
                vec![bucket_feature("accepted", i as u64)],
            );
        }
        assert_eq!(corpus.accepted.len(), CORPUS_SIZE);
        corpus.admit_rejected(ChoiceSequence::default(), vec![event_feature("rejected")]);
        // The combined size is capped: the rejected entry ties the
        // accepted single-feature entries on novel count, and ties
        // evict the earliest arrival, so the first accepted entry is
        // evicted and the rejected entry survives.
        assert_eq!(corpus.accepted.len() + corpus.rejected.len(), CORPUS_SIZE);
        assert_eq!(
            corpus.rejected.len(),
            1,
            "the rejected entry must survive the earliest-arrival tie-break"
        );
        assert!(
            !corpus
                .accepted
                .iter()
                .any(|e| e.novel.contains(&bucket_feature("accepted", 0))),
            "the earliest accepted entry must be evicted"
        );
    }

    // === CorpusGuidedSearch wiring ===

    #[test]
    fn corpus_guided_search_replays_accepted_entries() {
        // With a one-eighth restart probability, most picks are
        // exploratory: they replay the admitted draw under the same
        // declared constraint (possibly mutated, always in domain).
        // Recording (fresh) candidates draw unconstrained random bytes
        // instead. Deterministic per seed.
        let mut search = CorpusGuidedSearch::new(42);
        let mut seq = ChoiceSequence::default();
        seq.push_draw(
            7u64.to_le_bytes().to_vec(),
            ChoiceMeta::Bounded { bound: 10 },
        );
        search.corpus.admit_accepted(seq, vec![event_feature("a")]);
        let mut in_domain = 0;
        for _ in 0..16 {
            let mut ctx = search.next_context();
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
    fn corpus_guided_search_picks_from_rejected_queue_when_accepted_empty() {
        // While the accepted queue is empty, the rejected queue is the
        // only source for exploratory candidates (forced pick, no
        // rejected-queue roll consumed). The replayed draw must match
        // the admitted rejected sequence.
        let mut search = CorpusGuidedSearch::new(7);
        let mut seq = ChoiceSequence::default();
        seq.push_draw(
            3u64.to_le_bytes().to_vec(),
            ChoiceMeta::Bounded { bound: 10 },
        );
        search.corpus.admit_rejected(seq, vec![event_feature("r")]);
        let mut seen_replay = false;
        for _ in 0..16 {
            let mut ctx = search.next_context();
            ctx.set_next_choice_meta(ChoiceMeta::Bounded { bound: 10 });
            let mut buf = [0u8; 8];
            ctx.fill(&mut buf);
            let x = u64::from_le_bytes(buf);
            let _ = ctx.take_sequence();
            if x == 3 {
                seen_replay = true;
            }
        }
        assert!(
            seen_replay,
            "the rejected entry must be replayed while the accepted queue is empty"
        );
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

// === mutate_sequence ===

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

// === Stats corpus fields ===

#[test]
fn stats_corpus_fields_are_zero_for_uniform() {
    let mut runner = Runner::new(1);
    runner
        .run(4, |ctx| {
            crate::sample_u32(ctx);
            Ok(())
        })
        .unwrap();
    let stats = runner.stats();
    assert_eq!(stats.discovered_features, 0);
    assert_eq!(stats.max_corpus_size, 0);

    // The corpus fields stay 0 on failure paths too.
    let err = Runner::new(1)
        .run(4, |_ctx| {
            Err::<(), Box<dyn std::error::Error>>("boom".into())
        })
        .expect_err("returned Err must fail the run");
    assert_eq!(err.stats().discovered_features, 0);
    assert_eq!(err.stats().max_corpus_size, 0);

    let err = Runner::new(1)
        .run(4, |ctx| {
            crate::sample_u32(ctx);
            panic!("boom");
        })
        .expect_err("panicking closure must fail the run");
    assert_eq!(err.stats().discovered_features, 0);
    assert_eq!(err.stats().max_corpus_size, 0);
}

#[test]
fn stats_corpus_fields_reflect_observed_features() {
    // Every case reports the same feature, so exactly one feature is
    // observed and exactly one entry is admitted.
    let mut runner = Runner::new(1);
    runner
        .run_feedback_guided(4, |ctx| {
            ctx.bucket("b", 1);
            Ok(())
        })
        .unwrap();
    let stats = runner.stats();
    assert_eq!(stats.discovered_features, 1);
    assert_eq!(stats.max_corpus_size, 1);
}

#[test]
fn stats_corpus_fields_respect_corpus_cap() {
    // Every case reports a fresh feature: the corpus fills to the cap
    // while the observation set keeps growing. Each candidate invokes
    // the closure exactly once (exploratory replay replays draws, not
    // the closure), so 100 accepted cases register exactly 100
    // features.
    let case = std::cell::Cell::new(0u64);
    let mut runner = Runner::new(1);
    runner
        .run_feedback_guided(100, |ctx| {
            let i = case.get();
            case.set(i + 1);
            ctx.bucket("b", i);
            Ok(())
        })
        .unwrap();
    let stats = runner.stats();
    assert_eq!(stats.max_corpus_size, CORPUS_SIZE);
    assert_eq!(
        stats.discovered_features, 100,
        "the observation set must keep growing past the corpus cap"
    );
}

#[test]
fn stats_corpus_fields_on_too_many_rejections() {
    // Every case rejects, so the run ends with too-many-rejections
    // after the rejection cap. Each rejected case reports a fresh
    // feature before rejecting: with 103 cases the rejection cap
    // (1030) exceeds `MAX_GLOBAL_FEATURES`, so the observation set
    // saturates at the cap and the rejected queue fills to
    // `CORPUS_SIZE`; the error must carry both.
    let case = std::cell::Cell::new(0u64);
    let mut runner = Runner::new(1);
    let err = runner
        .run_feedback_guided(103, |ctx| {
            let i = case.get();
            case.set(i + 1);
            ctx.bucket("b", i);
            ctx.reject_case();
        })
        .expect_err("rejecting every case must hit the rejection cap");
    let stats = err.stats();
    assert_eq!(stats.rejected_cases, rejection_limit(103) + 1);
    assert_eq!(stats.discovered_features, MAX_GLOBAL_FEATURES);
    assert_eq!(stats.max_corpus_size, CORPUS_SIZE);
}

#[test]
fn corpus_stats_matches_corpus_state() {
    use crate::rng::{EventBucket, FeatureKind};
    let mut search = CorpusGuidedSearch::new(1);
    assert_eq!(corpus_stats(&search), (0, 0));

    // An accepted case registering a novel feature.
    let feature = Feature {
        label: "a",
        kind: FeatureKind::Event(EventBucket::One),
    };
    assert!(
        search
            .corpus
            .admit_accepted(ChoiceSequence::default(), vec![feature])
    );
    assert_eq!(corpus_stats(&search), (1, 1));

    // A rejected case registering another novel feature: rejected
    // cases also contribute to the observation set and the corpus.
    let feature = Feature {
        label: "b",
        kind: FeatureKind::Event(EventBucket::One),
    };
    assert!(
        search
            .corpus
            .admit_rejected(ChoiceSequence::default(), vec![feature])
    );
    assert_eq!(corpus_stats(&search), (2, 2));
}

#[test]
fn stats_corpus_fields_include_rejected_case_features() {
    // The first case rejects while reporting a novel feature: it
    // enters the rejected queue and its feature enters the observation
    // set. Later cases report the same (now observed) feature and are
    // not admitted.
    let case = std::cell::Cell::new(0u64);
    let mut runner = Runner::new(1);
    runner
        .run_feedback_guided(8, |ctx| {
            let i = case.get();
            case.set(i + 1);
            ctx.bucket("b", 1);
            if i == 0 {
                ctx.reject_case();
            }
            Ok(())
        })
        .expect("run must succeed");
    let stats = runner.stats();
    assert_eq!(stats.rejected_cases, 1);
    assert_eq!(stats.discovered_features, 1);
    assert_eq!(stats.max_corpus_size, 1);
}

#[test]
fn stats_corpus_fields_zero_cases() {
    let mut runner = Runner::new(1);
    runner
        .run_feedback_guided(0, |_ctx| {
            panic!("closure must not be invoked with zero cases");
        })
        .expect("zero cases must succeed");
    assert_eq!(runner.stats(), Stats::default());
}
