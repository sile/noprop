//! Non-cryptographic seedable PRNG (xoshiro256** with SplitMix64 seed expansion).

use std::any::Any;
use std::collections::VecDeque;
#[cfg(test)]
use std::panic::AssertUnwindSafe;
use std::panic::Location;

/// How many first values at the same location are kept verbatim in the
/// trace before elision starts.
const DEDUP_HEAD: usize = 8;

/// How many trailing values at the same location are kept in a rolling
/// buffer once the head is full.
const DEDUP_TAIL: usize = 8;

/// Maximum number of draws one case may *generate* during exploratory
/// replay. The first `MAX_CHOICES_PER_CASE` generated draws succeed;
/// the next generated draw aborts the case as a rejection so a mutated
/// candidate whose control flow keeps drawing cannot loop forever.
/// Replayed recorded draws are bounded by the recorded length and do
/// not count toward the cap.
const MAX_CHOICES_PER_CASE: usize = 4096;

/// Maximum number of semantic features one case may report in
/// corpus-guided mode. Exceeding the cap discards the excess; an event
/// saturating to a higher bucket replaces its earlier feature and does
/// not count as a new one.
const MAX_FEATURES_PER_CASE: usize = 64;

/// Seedable non-cryptographic PRNG used by all noprop generators.
///
/// The underlying algorithm is `xoshiro256**` with the initial 256-bit
/// state derived from a caller-supplied `u64` seed through `SplitMix64`
/// (Blackman/Vigna's recommended seeding procedure).
///
/// noprop never draws entropy from the OS or the system clock: the seed
/// must always be provided by the caller. This makes every property test
/// exactly reproducible from its seed.
///
/// The public methods are [`TestCaseContext::new`],
/// [`TestCaseContext::reject_case`], and — in targeted mode —
/// [`TestCaseContext::maximize`]; all byte/word production happens
/// through the `noprop::sample_*` free functions, which record the
/// generated values into an internal trace surfaced on failure. Raw
/// PRNG state access is deliberately hidden so users cannot
/// accidentally bypass that trace.
///
/// # Examples
///
/// ```
/// let mut ctx = noprop::TestCaseContext::new(0xDEAD_BEEF);
/// let a = noprop::sample_u32(&mut ctx);
/// let b = noprop::sample_u32(&mut ctx);
/// assert_ne!(a, b);
/// ```
pub struct TestCaseContext {
    source: RandomSource,
    generated: Vec<GeneratedValue>,
    dedup: DedupState,
    /// Set by [`TestCaseContext::reject_case`]. [`Runner::run`](crate::Runner::run)
    /// consults this after every case boundary and, if set, treats the
    /// iteration as rejected regardless of the closure's outcome.
    rejection: Option<RejectionState>,
    /// `true` when this `TestCaseContext` was constructed inside a
    /// [`Runner::run`](crate::Runner::run) invocation. `TestCaseContext::reject_case`
    /// checks this and panics with a Runner-only message when it is
    /// `false`, so the private control-flow marker is never sent from a
    /// context that cannot catch it.
    inside_runner: bool,
    /// Total number of [`record_generated`](Self::record_generated) calls
    /// observed across every case that ran on this context. Never reset
    /// between cases so [`Runner::run`](crate::Runner::run) can report it
    /// as [`Stats::total_samples`](crate::Stats::total_samples). Counted
    /// once per top-level `sample_*` invocation (dedup / elision happens
    /// after the counter is incremented, so folded runs are still fully
    /// counted).
    total_samples: usize,
    /// Scalar / semantic feedback collected during the current case.
    /// Always [`FeedbackState::Disabled`] when constructed via
    /// [`TestCaseContext::new`]; a runner switches it to `Targeted`
    /// before running its case loop.
    feedback: FeedbackState,
}

/// Private feedback state carried by every [`TestCaseContext`].
///
/// `Disabled` is the default and must stay allocation-free: `maximize`
/// and the semantic methods are no-ops while it is active. The
/// `Targeted` variant collects the maximum scalar reported via
/// [`TestCaseContext::maximize`] during one case; the
/// `SemanticCoverage` variant collects the semantic features reported
/// via [`TestCaseContext::event`] / `bucket` / `transition` and an
/// optional scalar priority via `maximize`.
#[derive(Debug, Clone)]
pub(crate) enum FeedbackState {
    Disabled,
    Targeted { max_score: ScalarFeedback },
    SemanticCoverage(SemanticCoverage),
}

/// Three-state scalar feedback for one case.
///
/// `Missing` marks an accepted case that never called `maximize`;
/// `Invalid` marks a `NaN` / infinity report. Keeping invalid out of
/// the `f64` payload means a later `Valid` score can never mask an
/// earlier invalid report.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum ScalarFeedback {
    #[default]
    Missing,
    Valid(f64),
    Invalid,
}

/// Per-case semantic feedback collected in corpus-guided mode.
///
/// `features` holds the features the case reported so far, capped at
/// [`MAX_FEATURES_PER_CASE`]; `priority` is the optional scalar score
/// reported via [`TestCaseContext::maximize`]. `event_counts` tracks
/// per-label repetition counts so repeated events saturate into
/// [`EventBucket`] features instead of unbounded counts.
#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticCoverage {
    features: Vec<Feature>,
    priority: ScalarFeedback,
    event_counts: Vec<(&'static str, usize)>,
}

impl SemanticCoverage {
    fn report_event(&mut self, label: &'static str) {
        let count = match self.event_counts.iter_mut().find(|(l, _)| *l == label) {
            Some((_, c)) => {
                *c += 1;
                *c
            }
            None => {
                self.event_counts.push((label, 1));
                1
            }
        };
        self.report(label, FeatureKind::Event(event_bucket(count)));
    }

    fn report(&mut self, label: &'static str, kind: FeatureKind) {
        // An event replaces the previous event feature with the same
        // label (bucket saturation); the replacement does not count
        // toward the per-case cap. Bucket / transition features
        // deduplicate on identity.
        if matches!(kind, FeatureKind::Event(_)) {
            if let Some(f) = self
                .features
                .iter_mut()
                .find(|f| f.label == label && matches!(f.kind, FeatureKind::Event(_)))
            {
                f.kind = kind;
                return;
            }
        } else if self
            .features
            .iter()
            .any(|f| f.label == label && f.kind == kind)
        {
            return;
        }
        if self.features.len() < MAX_FEATURES_PER_CASE {
            self.features.push(Feature { label, kind });
        }
    }

    fn report_priority(&mut self, score: f64) {
        let next = if score.is_finite() {
            match self.priority {
                ScalarFeedback::Missing => ScalarFeedback::Valid(score),
                ScalarFeedback::Valid(current) => ScalarFeedback::Valid(current.max(score)),
                ScalarFeedback::Invalid => ScalarFeedback::Invalid,
            }
        } else {
            ScalarFeedback::Invalid
        };
        self.priority = next;
    }

    /// The features reported during the case, in report order.
    #[cfg(test)]
    pub(crate) fn features(&self) -> &[Feature] {
        &self.features
    }

    /// Move the reported features out of the coverage state.
    pub(crate) fn take_features(&mut self) -> Vec<Feature> {
        std::mem::take(&mut self.features)
    }

    /// The optional scalar priority: `None` when `maximize` was never
    /// called or reported a `NaN` / infinity value. Both are tolerated
    /// in corpus-guided mode (the case proceeds without a priority).
    pub(crate) fn priority(&self) -> Option<f64> {
        match self.priority {
            ScalarFeedback::Valid(score) => Some(score),
            ScalarFeedback::Missing | ScalarFeedback::Invalid => None,
        }
    }
}

/// A semantic feature reported by a property in corpus-guided mode.
///
/// Feature identity is the `(label, kind)` pair: the same label used
/// for a different bucket or transition is a different feature.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Feature {
    pub label: &'static str,
    pub kind: FeatureKind,
}

impl Feature {
    /// Machine-free rendering for failure reports.
    pub(crate) fn display_repr(&self) -> String {
        match self.kind {
            FeatureKind::Event(_) => format!("event({:?})", self.label),
            FeatureKind::Bucket { value } => format!("bucket({:?}, {value})", self.label),
            FeatureKind::Transition { from, to } => {
                format!("transition({:?}, {from}, {to})", self.label)
            }
        }
    }
}

/// The kind of a semantic feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FeatureKind {
    /// Reaching a finite event. Repeated events saturate into
    /// [`EventBucket`] features.
    Event(EventBucket),
    /// A caller-bucketed finite state value.
    Bucket { value: u64 },
    /// An abstract state transition in a stateful test.
    Transition { from: u64, to: u64 },
}

/// Saturation buckets for repeated events within one case.
///
/// Counts 1, 2-3, 4-7, and 8+ map to distinct feature identities so a
/// case that visits an event many times is distinguished from one that
/// visits it once, without unbounded counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EventBucket {
    One,
    TwoThree,
    FourSeven,
    EightPlus,
}

/// Map a per-case event repetition count to its saturation bucket.
fn event_bucket(count: usize) -> EventBucket {
    match count {
        1 => EventBucket::One,
        2..=3 => EventBucket::TwoThree,
        4..=7 => EventBucket::FourSeven,
        _ => EventBucket::EightPlus,
    }
}

/// Private entropy source variant carried by every [`TestCaseContext`].
///
/// - `Prng` is the normal path — no per-draw allocation.
/// - `Recording` still drives the PRNG but also copies every non-empty
///   `fill` output into a [`ChoiceSequence`], and records nested attempt
///   spans opened by `sample_with_rejection`.
/// - `Replay` reads bytes only from a recorded sequence and reports a
///   [`ReplayError`] on structural mismatch, using a private
///   control-flow marker to abort the generator immediately. In
///   exploratory mode (`explore` is `Some`), draws beyond the recorded
///   sequence are generated from the carried PRNG instead of aborting,
///   and the generated draw count is capped by
///   [`MAX_CHOICES_PER_CASE`].
enum RandomSource {
    Prng(XoshiroState),
    Recording {
        state: XoshiroState,
        sequence: ChoiceSequence,
        /// Index into `sequence.spans` of the currently in-flight
        /// attempt, or `None` when no attempt is open (top level).
        current_parent: Option<usize>,
        /// Metadata to attach to the next draw (set by bounded-domain
        /// primitives right before they draw).
        pending_choice: Option<ChoiceMeta>,
    },
    Replay {
        sequence: ChoiceSequence,
        next_draw: usize,
        next_span: usize,
        /// Index into `sequence.spans` of the currently in-flight
        /// attempt, or `None` when no attempt is open.
        current_parent: Option<usize>,
        error: Option<ReplayError>,
        /// `Some` when running exploratory replay: draws beyond the
        /// recorded sequence are generated from this PRNG instead of
        /// aborting, capped at [`MAX_CHOICES_PER_CASE`] generated
        /// draws.
        explore: Option<XoshiroState>,
        /// Generated draws so far in this exploratory case, used to
        /// enforce the [`MAX_CHOICES_PER_CASE`] cap. Replayed recorded
        /// draws are bounded by the recorded length and do not count.
        consumed: usize,
        /// Metadata to attach to the next generated draw (set by
        /// bounded-domain primitives right before they draw).
        pending_choice: Option<ChoiceMeta>,
    },
}

/// Private `xoshiro256**` state and step function used by both `Prng`
/// and `Recording` sources.
///
/// Kept out of [`TestCaseContext`] itself so `TestCaseContext` has only one entropy boundary
/// ([`TestCaseContext::fill`]) — Recording / Replay cannot expose a separate raw
/// `next_u64` path with mode-dependent semantics.
pub(crate) struct XoshiroState {
    state: [u64; 4],
}

impl XoshiroState {
    pub(crate) fn from_seed(seed: u64) -> Self {
        let mut sm = SplitMix64 { state: seed };
        Self {
            state: [sm.next(), sm.next(), sm.next(), sm.next()],
        }
    }

    /// Advance the PRNG by one step and return the next 64 random bits.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let result = self.state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);

        let t = self.state[1] << 17;

        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];

        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);

        result
    }

    pub(crate) fn fill(&mut self, dst: &mut [u8]) {
        let mut i = 0;
        while i + 8 <= dst.len() {
            let bytes = self.next_u64().to_le_bytes();
            dst[i..i + 8].copy_from_slice(&bytes);
            i += 8;
        }
        if i < dst.len() {
            let bytes = self.next_u64().to_le_bytes();
            let remaining = dst.len() - i;
            dst[i..].copy_from_slice(&bytes[..remaining]);
        }
    }

    /// Uniform value in `[0, n)` using rejection sampling directly on
    /// the PRNG stream. Mirrors `sample_below` but operates on the
    /// carried state instead of a `TestCaseContext`.
    ///
    /// Unlike the generator-side sampler there is no attempt cap: the
    /// loop is expected to terminate within two attempts on average
    /// (the acceptance region covers at least half of the range), and
    /// `n > 0` is guaranteed by callers.
    pub(crate) fn sample_below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "sample_below: n must be non-zero");
        if n == 1 {
            return 0;
        }
        let r = u64::MAX % n;
        if r == n - 1 {
            return self.next_u64() % n;
        }
        let bound = u64::MAX - r;
        loop {
            let x = self.next_u64();
            if x < bound {
                return x % n;
            }
        }
    }
}

/// Bytes and attempt spans consumed by a single case, in call order.
///
/// - Each non-empty [`TestCaseContext::fill`] call is one entry in `draws`.
/// - Each `sample_with_rejection` attempt (including nested ones) is
///   one entry in `spans`.
/// - Each draw's bounded-domain metadata (if any) is one entry in
///   `metas`, parallel to `draws`.
///
/// Empty `fill` calls neither consume PRNG state nor produce a draw
/// entry, matching the existing `TestCaseContext::fill(&mut [])` contract.
#[derive(Default, Clone)]
pub(crate) struct ChoiceSequence {
    draws: Vec<Vec<u8>>,
    spans: Vec<AttemptSpan>,
    metas: Vec<ChoiceMeta>,
}

impl ChoiceSequence {
    #[cfg(test)]
    pub(crate) fn draws(&self) -> &[Vec<u8>] {
        &self.draws
    }

    #[cfg(test)]
    pub(crate) fn spans(&self) -> &[AttemptSpan] {
        &self.spans
    }

    #[cfg(test)]
    pub(crate) fn metas(&self) -> &[ChoiceMeta] {
        &self.metas
    }

    #[cfg(test)]
    pub(crate) fn draws_mut(&mut self) -> &mut [Vec<u8>] {
        &mut self.draws
    }

    /// Split borrow of draws (mutable) and their metadata (shared), so
    /// mutation can walk both at once.
    pub(crate) fn draws_and_metas(&mut self) -> (&mut [Vec<u8>], &[ChoiceMeta]) {
        (&mut self.draws, &self.metas)
    }

    pub(crate) fn push_draw(&mut self, bytes: Vec<u8>, meta: ChoiceMeta) {
        self.draws.push(bytes);
        self.metas.push(meta);
    }
}

/// Bounded-domain metadata for one draw.
///
/// The variants carry enough information for constraint-aware
/// mutation: a `Bounded` draw may only be rewritten to another value
/// in `[0, bound)`, a `Choice` draw to another index in `[0, len)`, an
/// `Integer` draw (plain `sample_u*` / `sample_i*`) to any value of its
/// width. `Raw` marks a draw with no mutation constraint (raw bytes,
/// string payload, floats, …), which is regenerated as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChoiceMeta {
    /// Drawn uniformly from `[0, bound)` via the rejection-sampling
    /// core (`sample_below`).
    Bounded { bound: u64 },
    /// Drawn as an index into a candidate slice of length `len`
    /// (`sample_choice`).
    Choice { len: usize },
    /// A plain integer draw (`sample_u*` / `sample_i*`) over its full
    /// width. Mutation may rewrite it to any value of the same width.
    Integer,
    /// No bounded domain (raw bytes, string payload, …).
    Raw,
}

/// One `sample_with_rejection` attempt, recorded in the order the
/// enclosing case opened it. Nested attempts point back to their parent
/// via `parent`; top-level attempts carry `parent = None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptSpan {
    pub parent: Option<usize>,
    pub start_draw: usize,
    pub end_draw: usize,
    pub verdict: AttemptVerdict,
}

/// Outcome the `sample_with_rejection` attempt returned. `Pending` is
/// only observable during recording while the closure is still running;
/// it is replaced by `Accepted` / `Rejected` before the span is
/// finalized, so it never appears in a released `ChoiceSequence`.
///
/// Kept out of `#[cfg(test)]` because `sample_with_rejection` (a
/// public production function) needs to pass this to
/// `TestCaseContext::end_attempt`; the Prng branch discards it, so it costs
/// nothing at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptVerdict {
    Pending,
    Accepted,
    Rejected,
}

/// Reason a strict replay could not reproduce the recorded case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplayError {
    /// The generator asked for another draw but the sequence had no more entries.
    SequenceExhausted { requested: usize },
    /// The next recorded draw's length does not match the requested length.
    DrawLengthMismatch { expected: usize, actual: usize },
    /// The generator returned but recorded draws remain unread.
    #[cfg_attr(not(test), expect(dead_code))]
    LeftoverDraws { unused: usize },
    /// The attempt span structure (parent nesting, draw range, or
    /// verdict) diverged from the recorded sequence — distinct from
    /// byte-level mismatches so tests can pattern-match on the kind of
    /// divergence.
    SpanMismatch {
        at_span: usize,
        reason: SpanMismatchReason,
    },
    /// The generator returned but recorded spans remain unread.
    #[cfg_attr(not(test), expect(dead_code))]
    LeftoverSpans { unused: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpanMismatchReason {
    /// The generator opened or closed a span but the recorded sequence
    /// had no matching span at this position.
    SequenceExhausted,
    /// Nesting differed (unexpected parent for this span).
    ParentMismatch,
    /// Draw offset at attempt start differed from the recording.
    StartDrawMismatch { expected: usize, actual: usize },
    /// Draw offset at attempt end differed from the recording.
    EndDrawMismatch { expected: usize, actual: usize },
    /// Accepted vs Rejected verdict differed from the recording.
    VerdictMismatch {
        expected: AttemptVerdict,
        actual: AttemptVerdict,
    },
}

/// Private control-flow marker used to abort a generator on the first
/// structural replay mismatch. Sent via [`std::panic::resume_unwind`]
/// so it bypasses the panic hook, and identified back in the session
/// via `Any::is::<ReplayAbort>()`.
struct ReplayAbort;

/// Private control-flow marker used by [`TestCaseContext::reject_case`] to unwind
/// out of the property closure. [`Runner::run`](crate::Runner::run)
/// catches it and, in combination with the `rejection` state saved on
/// the [`TestCaseContext`], treats the iteration as rejected. Never sent from a
/// context that lacks the catching boundary — `reject_case` checks
/// `TestCaseContext::inside_runner` first.
pub(crate) struct IterationRejected;

/// Diagnostic state saved by [`TestCaseContext::reject_case`] before it unwinds.
/// Runner reads this after `catch_unwind` and reports it on
/// [`TooManyRejections`](crate::Error) failures.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RejectionState {
    pub location: &'static Location<'static>,
}

/// A single entry in the generated-value trace collected during a case.
///
/// Entries fall into two kinds, distinguishable via [`is_elided`](Self::is_elided):
///
/// - **Value entries** carry a `Debug`-formattable value at a source
///   location. The Rust type name and formatted representation are
///   available via [`type_name`](Self::type_name) and
///   [`value_repr`](Self::value_repr).
/// - **Elision markers** stand in for a run of same-location entries
///   that were skipped to keep the trace compact. See the docs on
///   [`Error::generated`](crate::Error::generated) for the elision
///   policy, and use [`elided_count`](Self::elided_count) to inspect.
pub struct GeneratedValue {
    type_name: &'static str,
    kind: EntryKind,
    location: &'static Location<'static>,
}

enum EntryKind {
    Value(Box<dyn std::fmt::Debug + 'static>),
    Elided { count: usize },
}

impl GeneratedValue {
    /// The Rust type name of the entry (for elision markers, the type
    /// of the values that were elided).
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Source location at which the generator was called.
    pub fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// `Debug`-formatted string of the generated value, or `None` if
    /// this entry is an elision marker.
    pub fn value_repr(&self) -> Option<String> {
        match &self.kind {
            EntryKind::Value(v) => Some(format!("{v:?}")),
            EntryKind::Elided { .. } => None,
        }
    }

    /// True if this entry marks a run of skipped same-location values.
    pub fn is_elided(&self) -> bool {
        matches!(self.kind, EntryKind::Elided { .. })
    }

    /// Number of skipped values, or `None` if this is a value entry.
    pub fn elided_count(&self) -> Option<usize> {
        if let EntryKind::Elided { count } = self.kind {
            Some(count)
        } else {
            None
        }
    }
}

impl std::fmt::Debug for GeneratedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            EntryKind::Value(v) => write!(
                f,
                "- {} = {v:?}  (at {}:{})",
                self.type_name,
                self.location.file(),
                self.location.line(),
            ),
            EntryKind::Elided { count } => write!(
                f,
                "... {count} more {} elided at {}:{} ...",
                self.type_name,
                self.location.file(),
                self.location.line(),
            ),
        }
    }
}

/// Per-location deduplication state for the generated-value trace.
///
/// Compared by [`Location`] (file/line/column) only — the value's type
/// is not part of the equality check, so a loop that produces multiple
/// types at the same call site (typical for `#[track_caller]`-propagated
/// user-defined generators) still deduplicates correctly.
#[derive(Default)]
struct DedupState {
    location: Option<&'static Location<'static>>,
    head_count: usize,
    tail: VecDeque<GeneratedValue>,
    elided: usize,
}

/// Charge one generated draw against the per-case exploratory cap.
///
/// Both generation paths of [`TestCaseContext::fill`] (dead-draw
/// replacement and append past the recording) share this accounting:
/// returns `false` when the cap is exceeded, with the case marked as a
/// rejection so the caller unwinds it.
fn within_generated_draw_cap(consumed: &mut usize, rejection: &mut Option<RejectionState>) -> bool {
    *consumed += 1;
    if *consumed > MAX_CHOICES_PER_CASE {
        *rejection = Some(RejectionState {
            location: std::panic::Location::caller(),
        });
        return false;
    }
    true
}

impl TestCaseContext {
    /// Create a new [`TestCaseContext`] from a 64-bit seed.
    ///
    /// The seed is expanded to the 256-bit internal state through
    /// SplitMix64. Passing the same seed twice always produces the same
    /// output stream.
    pub fn new(seed: u64) -> Self {
        Self {
            source: RandomSource::Prng(XoshiroState::from_seed(seed)),
            generated: Vec::new(),
            dedup: DedupState::default(),
            rejection: None,
            inside_runner: false,
            total_samples: 0,
            feedback: FeedbackState::Disabled,
        }
    }

    /// Construct a context in recording mode for the targeted runner.
    /// Every draw is appended to the carried [`ChoiceSequence`], which
    /// the runner recovers via [`take_sequence`](Self::take_sequence)
    /// at the case boundary.
    pub(crate) fn recording(seed: u64) -> Self {
        Self {
            source: RandomSource::Recording {
                state: XoshiroState::from_seed(seed),
                sequence: ChoiceSequence::default(),
                current_parent: None,
                pending_choice: None,
            },
            generated: Vec::new(),
            dedup: DedupState::default(),
            rejection: None,
            inside_runner: false,
            total_samples: 0,
            feedback: FeedbackState::Disabled,
        }
    }

    /// Construct a context in exploratory replay mode for a mutated
    /// candidate. Draws within the recorded sequence are replayed
    /// verbatim; draws beyond it are generated from the carried PRNG
    /// (seeded from `prng_seed`), up to [`MAX_CHOICES_PER_CASE`].
    pub(crate) fn exploring(sequence: ChoiceSequence, prng_seed: u64) -> Self {
        Self {
            source: RandomSource::Replay {
                sequence,
                next_draw: 0,
                next_span: 0,
                current_parent: None,
                error: None,
                explore: Some(XoshiroState::from_seed(prng_seed)),
                consumed: 0,
                pending_choice: None,
            },
            generated: Vec::new(),
            dedup: DedupState::default(),
            rejection: None,
            inside_runner: false,
            total_samples: 0,
            feedback: FeedbackState::Disabled,
        }
    }

    /// Recover the recorded choice sequence from a recording-mode or
    /// exploratory-replay context, leaving an empty sequence behind.
    ///
    /// For exploratory contexts, recorded draws the mutated control
    /// flow never consumed (the "unconsumed suffix") are discarded so
    /// stale values do not leak into the next generation. Spans are
    /// dropped entirely: the recorded structure is only a mutation
    /// seed, and a truncated span list would dangle (a span's parent
    /// may have been cut). Nothing consumes exploratory spans today.
    pub(crate) fn take_sequence(&mut self) -> Option<ChoiceSequence> {
        match &mut self.source {
            RandomSource::Recording { sequence, .. } => Some(std::mem::take(sequence)),
            RandomSource::Replay {
                sequence,
                next_draw,
                ..
            } => {
                if *next_draw < sequence.draws.len() {
                    sequence.draws.truncate(*next_draw);
                    sequence.metas.truncate(*next_draw);
                }
                sequence.spans.clear();
                Some(std::mem::take(sequence))
            }
            _ => None,
        }
    }

    /// Reject the current iteration and unwind out of the property
    /// closure. Only valid inside [`Runner::run`](crate::Runner::run) or
    /// [`Runner::run_targeted`](crate::Runner::run_targeted);
    /// calling this from a `TestCaseContext` constructed outside a runner panics
    /// with a Runner-only message.
    ///
    /// Prefer
    /// [`sample_with_rejection`](crate::sample_with_rejection) as the
    /// high-level bounded-retry helper. Reach for `reject_case`
    /// directly only when the whole iteration turns out to be an
    /// unsuitable candidate *after* sampling has finished and the
    /// helper cannot express the exit.
    ///
    /// The runner discards the current iteration's generated-value
    /// trace and tries the next iteration. Rejected iterations are not
    /// counted toward
    /// `iterations`; rejected +
    /// accepted attempts together are bounded by an internal global
    /// limit so a generator that always rejects still terminates.
    #[track_caller]
    pub fn reject_case(&mut self) -> ! {
        if !self.inside_runner {
            panic!(
                "noprop::TestCaseContext::reject_case can only be called from inside a Runner::run or \
                 Runner::run_targeted property closure. Constructing a TestCaseContext directly via TestCaseContext::new does not \
                 create a Runner boundary."
            );
        }
        let location = Location::caller();
        self.rejection = Some(RejectionState { location });
        std::panic::resume_unwind(Box::new(IterationRejected));
    }

    /// Fill `dst` with random bytes. An empty slice consumes no RNG
    /// state and produces no [`ChoiceSequence`] entry.
    ///
    /// This is the single entropy boundary of [`TestCaseContext`]. In
    /// `Recording` mode each non-empty call is copied into the sequence
    /// in order. In `Replay` mode the behavior depends on the
    /// exploratory flag:
    ///
    /// * strict replay (no exploratory flag) reads bytes from the
    ///   recorded sequence; a request past the recording, or one with a
    ///   different width, aborts via a private control-flow marker;
    /// * exploratory replay serves a same-width request from the
    ///   recording — regenerating the value when the primitive
    ///   constraint changed at that position — and answers a request
    ///   past the recording, or one with a different width, with a
    ///   freshly generated draw. Generated draws count toward
    ///   [`MAX_CHOICES_PER_CASE`]; a case exceeding the cap is rejected
    ///   via the private control-flow marker.
    ///
    /// `pub(crate)` — see the [`TestCaseContext`] type-level docs for why.
    pub(crate) fn fill(&mut self, dst: &mut [u8]) {
        if dst.is_empty() {
            return;
        }
        match &mut self.source {
            RandomSource::Prng(state) => state.fill(dst),
            RandomSource::Recording {
                state,
                sequence,
                pending_choice,
                ..
            } => {
                state.fill(dst);
                let meta = pending_choice.take().unwrap_or(ChoiceMeta::Raw);
                sequence.push_draw(dst.to_vec(), meta);
            }
            RandomSource::Replay {
                sequence,
                next_draw,
                error,
                explore,
                consumed,
                pending_choice,
                ..
            } => {
                if error.is_none() {
                    if *next_draw < sequence.draws.len() {
                        let draw_len = sequence.draws[*next_draw].len();
                        if draw_len == dst.len() {
                            if explore.is_some() {
                                // The mutated control flow may read
                                // this draw position under a different
                                // constraint (a different primitive or
                                // bound at the same width), or drop the
                                // constraint entirely (a metadata-free
                                // primitive). Per the exploratory
                                // replay rule, regenerate the value at
                                // a changed constraint before replaying
                                // it, so the executed value always
                                // matches the stored value.
                                let meta = pending_choice.take().unwrap_or(ChoiceMeta::Raw);
                                let idx = *next_draw;
                                if sequence.metas[idx] != meta {
                                    if let Some(prng) = explore {
                                        prng.fill(&mut sequence.draws[idx]);
                                    }
                                    sequence.metas[idx] = meta;
                                }
                            }
                            dst.copy_from_slice(&sequence.draws[*next_draw]);
                            *next_draw += 1;
                            return;
                        }
                        if explore.is_none() {
                            *error = Some(ReplayError::DrawLengthMismatch {
                                expected: draw_len,
                                actual: dst.len(),
                            });
                            std::panic::resume_unwind(Box::new(ReplayAbort));
                        }
                        // Exploratory: the mutated control flow asks
                        // for a different width, so this recorded draw
                        // is dead — replace it in place with a fresh
                        // generated draw so the sequence stays bounded.
                        if let Some(prng) = explore {
                            if !within_generated_draw_cap(consumed, &mut self.rejection) {
                                std::panic::resume_unwind(Box::new(IterationRejected));
                            }
                            prng.fill(dst);
                            let meta = pending_choice.take().unwrap_or(ChoiceMeta::Raw);
                            sequence.draws[*next_draw] = dst.to_vec();
                            sequence.metas[*next_draw] = meta;
                            *next_draw += 1;
                            return;
                        }
                    } else if explore.is_none() {
                        *error = Some(ReplayError::SequenceExhausted {
                            requested: dst.len(),
                        });
                        std::panic::resume_unwind(Box::new(ReplayAbort));
                    }
                    // Recorded draws exhausted: generate and append.
                    if let Some(prng) = explore {
                        if !within_generated_draw_cap(consumed, &mut self.rejection) {
                            // The mutated candidate keeps drawing past
                            // the cap: abort the case as a rejection so
                            // the run still terminates.
                            std::panic::resume_unwind(Box::new(IterationRejected));
                        }
                        prng.fill(dst);
                        // Record the generated draw (with the bounded
                        // metadata the primitive declared, if any) so an
                        // accepted candidate can re-enter the corpus and
                        // its tail remains mutable.
                        let meta = pending_choice.take().unwrap_or(ChoiceMeta::Raw);
                        sequence.push_draw(dst.to_vec(), meta);
                        *next_draw += 1;
                        return;
                    }
                }
                std::panic::resume_unwind(Box::new(ReplayAbort));
            }
        }
    }

    /// Open a new attempt span. In `Prng` mode this is a no-op returning
    /// `None`; in `Recording` mode it records a placeholder span linked
    /// to the current parent; in `Replay` mode it validates the span
    /// against the recording. The returned index (in Recording) is fed
    /// back to `end_attempt`.
    pub(crate) fn begin_attempt(&mut self) -> Option<usize> {
        match &mut self.source {
            RandomSource::Prng(_) => None,
            RandomSource::Recording {
                sequence,
                current_parent,
                ..
            } => {
                let idx = sequence.spans.len();
                let start_draw = sequence.draws.len();
                sequence.spans.push(AttemptSpan {
                    parent: *current_parent,
                    start_draw,
                    end_draw: start_draw,
                    verdict: AttemptVerdict::Pending,
                });
                *current_parent = Some(idx);
                Some(idx)
            }
            RandomSource::Replay {
                sequence,
                next_draw,
                next_span,
                current_parent,
                error,
                explore,
                ..
            } => {
                if explore.is_some() {
                    // Exploratory mode: a mutated candidate's control
                    // flow may legitimately diverge from the recorded
                    // span structure. Spans are not recorded here — the
                    // recorded structure is only a seed for the next
                    // mutation, and nothing consumes exploratory spans.
                    return None;
                }
                if error.is_some() {
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                let idx = *next_span;
                if idx >= sequence.spans.len() {
                    *error = Some(ReplayError::SpanMismatch {
                        at_span: idx,
                        reason: SpanMismatchReason::SequenceExhausted,
                    });
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                let recorded = &sequence.spans[idx];
                if recorded.parent != *current_parent {
                    *error = Some(ReplayError::SpanMismatch {
                        at_span: idx,
                        reason: SpanMismatchReason::ParentMismatch,
                    });
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                if recorded.start_draw != *next_draw {
                    *error = Some(ReplayError::SpanMismatch {
                        at_span: idx,
                        reason: SpanMismatchReason::StartDrawMismatch {
                            expected: recorded.start_draw,
                            actual: *next_draw,
                        },
                    });
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                *next_span += 1;
                *current_parent = Some(idx);
                Some(idx)
            }
        }
    }

    /// Close the attempt opened by `begin_attempt`. `id` is what
    /// `begin_attempt` returned; `verdict` is `Accepted` or `Rejected`.
    /// In `Prng` mode this is a no-op (`id` is always `None`).
    pub(crate) fn end_attempt(&mut self, id: Option<usize>, verdict: AttemptVerdict) {
        debug_assert!(!matches!(verdict, AttemptVerdict::Pending));
        match &mut self.source {
            RandomSource::Prng(_) => {
                let _ = id;
                let _ = verdict;
            }
            RandomSource::Recording {
                sequence,
                current_parent,
                ..
            } => {
                let idx = id.expect("Recording mode always yields an attempt id");
                let end_draw = sequence.draws.len();
                let span = &mut sequence.spans[idx];
                span.end_draw = end_draw;
                span.verdict = verdict;
                *current_parent = span.parent;
            }
            RandomSource::Replay {
                sequence,
                next_draw,
                current_parent,
                error,
                explore,
                ..
            } => {
                if explore.is_some() {
                    // Exploratory mode: `begin_attempt` returned
                    // `None`, so there is nothing to close.
                    return;
                }
                if error.is_some() {
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                let idx = id.expect("Replay mode always yields an attempt id");
                let recorded = &sequence.spans[idx];
                if recorded.end_draw != *next_draw {
                    *error = Some(ReplayError::SpanMismatch {
                        at_span: idx,
                        reason: SpanMismatchReason::EndDrawMismatch {
                            expected: recorded.end_draw,
                            actual: *next_draw,
                        },
                    });
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                if recorded.verdict != verdict {
                    *error = Some(ReplayError::SpanMismatch {
                        at_span: idx,
                        reason: SpanMismatchReason::VerdictMismatch {
                            expected: recorded.verdict,
                            actual: verdict,
                        },
                    });
                    std::panic::resume_unwind(Box::new(ReplayAbort));
                }
                *current_parent = recorded.parent;
            }
        }
    }

    /// Consume and return any pending rejection state saved by
    /// [`TestCaseContext::reject_case`]. Called by
    /// [`Runner::run`](crate::Runner::run) and
    /// [`Runner::run_targeted`](crate::Runner::run_targeted) after each case boundary so
    /// a set state wins over the closure's own `Ok` / `Err` / non-marker
    /// panic outcome.
    pub(crate) fn take_rejection(&mut self) -> Option<RejectionState> {
        self.rejection.take()
    }

    /// Record a generated value in this TestCaseContext's buffer. Called from every
    /// primitive generator right after producing the value.
    ///
    /// `location` should be `std::panic::Location::caller()` captured at
    /// the top of the primitive (which itself carries `#[track_caller]`),
    /// so that the recorded position points at the call site in the
    /// user's property closure rather than at the primitive's own body.
    ///
    /// The value is stored `Box<dyn Debug>` (format-lazy): its Debug
    /// representation is only rendered if the trace is actually
    /// displayed at failure time, not on every generator call.
    ///
    /// A run of many same-location entries is deduplicated: the first
    /// 8 and last 8 are kept verbatim; the middle is replaced with a
    /// single elision-marker entry that carries the skipped count.
    /// Comparison is by location only, so a `#[track_caller]`-propagated
    /// composite generator (e.g. a user-defined `sample_person` called
    /// inside a loop) also folds correctly.
    #[track_caller]
    pub(crate) fn record_generated<T: std::fmt::Debug + Clone + 'static>(
        &mut self,
        value: &T,
        location: &'static Location<'static>,
    ) {
        self.total_samples = self.total_samples.saturating_add(1);

        let same_as_last = self
            .dedup
            .location
            .is_some_and(|l| same_location(l, location));

        if !same_as_last {
            self.flush_dedup_tail();
            self.dedup.location = Some(location);
            self.dedup.head_count = 0;
            // `elided` and `tail` are drained/reset in `flush_dedup_tail`.
        }

        let entry = GeneratedValue {
            type_name: std::any::type_name::<T>(),
            kind: EntryKind::Value(Box::new(value.clone())),
            location,
        };

        if self.dedup.head_count < DEDUP_HEAD {
            self.generated.push(entry);
            self.dedup.head_count += 1;
        } else {
            if self.dedup.tail.len() == DEDUP_TAIL {
                self.dedup.tail.pop_front();
                self.dedup.elided += 1;
            }
            self.dedup.tail.push_back(entry);
        }
    }

    /// Flush the trailing tail buffer for the current same-location run
    /// into [`generated`](Self::generated), preceded by an elision
    /// marker if any entries were dropped.
    fn flush_dedup_tail(&mut self) {
        if self.dedup.elided > 0 {
            let location = self
                .dedup
                .location
                .expect("elided implies a prior location");
            let type_name = self.dedup.tail.front().map(|e| e.type_name).unwrap_or("?");
            self.generated.push(GeneratedValue {
                type_name,
                kind: EntryKind::Elided {
                    count: self.dedup.elided,
                },
                location,
            });
        }
        self.generated.extend(self.dedup.tail.drain(..));
        self.dedup.elided = 0;
    }

    pub(crate) fn take_generated(&mut self) -> Vec<GeneratedValue> {
        self.flush_dedup_tail();
        self.dedup = DedupState::default();
        std::mem::take(&mut self.generated)
    }

    pub(crate) fn clear_generated(&mut self) {
        self.generated.clear();
        self.dedup = DedupState::default();
    }

    /// Enable the Runner-only guard on
    /// [`TestCaseContext::reject_case`]. Called by
    /// [`Runner::run`](crate::Runner::run) or
    /// [`Runner::run_targeted`](crate::Runner::run_targeted) immediately after
    /// constructing its `TestCaseContext`.
    pub(crate) fn set_inside_runner(&mut self) {
        self.inside_runner = true;
    }

    /// Report a scalar "distance to failure" for the current case.
    ///
    /// Only meaningful in targeted mode: [`Runner::run_targeted`](crate::Runner::run_targeted)
    /// switches the context into that mode before running its case
    /// loop. In the default mode (plain [`Runner::run`](crate::Runner::run)
    /// or a directly constructed context) this is an allocation-free
    /// no-op.
    ///
    /// A larger finite value means "closer to failure". Multiple calls
    /// within one case keep the maximum; a `NaN` / infinity report
    /// marks the whole case invalid. Rejected cases (via
    /// [`TestCaseContext::reject_case`]) discard their score — they do
    /// not need to call `maximize`.
    ///
    /// In corpus-guided mode (see
    /// [`Runner::run_corpus_guided`](crate::Runner::run_corpus_guided))
    /// this reports an optional scalar priority for the case, using the
    /// same aggregation and invalid handling.
    pub fn maximize(&mut self, score: f64) {
        match &mut self.feedback {
            FeedbackState::Targeted { max_score } => {
                let next = if score.is_finite() {
                    match *max_score {
                        ScalarFeedback::Missing => ScalarFeedback::Valid(score),
                        ScalarFeedback::Valid(current) => ScalarFeedback::Valid(current.max(score)),
                        ScalarFeedback::Invalid => ScalarFeedback::Invalid,
                    }
                } else {
                    ScalarFeedback::Invalid
                };
                *max_score = next;
            }
            FeedbackState::SemanticCoverage(cov) => cov.report_priority(score),
            FeedbackState::Disabled => {}
        }
    }

    /// Report reaching a finite event for the current case.
    ///
    /// Only meaningful in corpus-guided mode:
    /// [`Runner::run_corpus_guided`](crate::Runner::run_corpus_guided)
    /// switches the context into that mode before running its case
    /// loop. In the default mode (plain [`Runner::run`](crate::Runner::run)
    /// or a directly constructed context) this is an allocation-free
    /// no-op.
    ///
    /// Repeating the same event within one case saturates into a fixed
    /// hit-count bucket (1 / 2-3 / 4-7 / 8+ occurrences), so a case
    /// that visits an event many times is distinguished from one that
    /// visits it once, without unbounded counts.
    pub fn event(&mut self, label: &'static str) {
        if let FeedbackState::SemanticCoverage(cov) = &mut self.feedback {
            cov.report_event(label);
        }
    }

    /// Report a caller-bucketed finite state value for the current
    /// case. `value` must come from a finite bucket designed by the
    /// caller; unbounded values (timestamps, sequence numbers, byte
    /// counts) defeat the corpus's stability and size bounds.
    ///
    /// Only meaningful in corpus-guided mode; a no-op otherwise.
    /// Reporting the same `(label, value)` pair again within one case
    /// is deduplicated.
    pub fn bucket(&mut self, label: &'static str, value: u64) {
        if let FeedbackState::SemanticCoverage(cov) = &mut self.feedback {
            cov.report(label, FeatureKind::Bucket { value });
        }
    }

    /// Report an abstract state transition for the current case: the
    /// stateful test's model moved from `from` to `to` under the
    /// command named by `label`.
    ///
    /// Only meaningful in corpus-guided mode; a no-op otherwise.
    /// Reporting the same transition again within one case is
    /// deduplicated.
    pub fn transition(&mut self, label: &'static str, from: u64, to: u64) {
        if let FeedbackState::SemanticCoverage(cov) = &mut self.feedback {
            cov.report(label, FeatureKind::Transition { from, to });
        }
    }

    /// Switch the context into targeted feedback mode for the upcoming
    /// case. Called by [`Runner::run_targeted`](crate::Runner::run_targeted)
    /// before each case; the case-local score is drained via
    /// [`take_feedback`](Self::take_feedback) at the case boundary.
    pub(crate) fn enable_targeted(&mut self) {
        self.feedback = FeedbackState::Targeted {
            max_score: ScalarFeedback::Missing,
        };
    }

    /// Switch the context into corpus-guided feedback mode for the
    /// upcoming case. Called by
    /// [`Runner::run_corpus_guided`](crate::Runner::run_corpus_guided)
    /// before each case; the case-local feedback is drained via
    /// [`take_feedback`](Self::take_feedback) at the case boundary.
    pub(crate) fn enable_corpus_guided(&mut self) {
        self.feedback = FeedbackState::SemanticCoverage(SemanticCoverage::default());
    }

    /// Drain the case-local feedback state, resetting to disabled.
    pub(crate) fn take_feedback(&mut self) -> FeedbackState {
        std::mem::replace(&mut self.feedback, FeedbackState::Disabled)
    }

    /// Declare the bounded domain of the next draw. Consumed by
    /// [`fill`](Self::fill) in Recording mode and by the exploratory
    /// generation path; ignored otherwise.
    pub(crate) fn set_next_choice_meta(&mut self, meta: ChoiceMeta) {
        match &mut self.source {
            RandomSource::Recording { pending_choice, .. }
            | RandomSource::Replay { pending_choice, .. } => {
                *pending_choice = Some(meta);
            }
            RandomSource::Prng(_) => {}
        }
    }

    /// Total number of top-level `sample_*` invocations observed on this
    /// context across every case that has run so far. Consumed by
    /// [`Runner::run`](crate::Runner::run) and
    /// [`Runner::run_targeted`](crate::Runner::run_targeted) when they build
    /// [`Stats`](crate::Stats).
    pub(crate) fn total_samples(&self) -> usize {
        self.total_samples
    }
}

/// Session that records a case's [`ChoiceSequence`] from a seed.
///
/// The session owns the `TestCaseContext` handed to the closure and returns it
/// alongside the closure's own return value. Recording is not woven
/// into `TestCaseContext::new` or `Runner::run` because those production paths
/// have no consumer for the sequence — [`RecordingSession`] is the
/// only entry point that allocates one outside the targeted runner.
#[cfg(test)]
pub(crate) struct RecordingSession {
    seed: u64,
}

#[cfg(test)]
impl RecordingSession {
    pub(crate) fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub(crate) fn run<T, F>(self, f: F) -> (T, ChoiceSequence)
    where
        F: FnOnce(&mut TestCaseContext) -> T,
    {
        let mut ctx = TestCaseContext::recording(self.seed);
        let value = f(&mut ctx);
        let sequence = ctx
            .take_sequence()
            .expect("recording mode always yields a sequence");
        (value, sequence)
    }
}

/// Session that strictly replays a recorded [`ChoiceSequence`].
///
/// Generator execution and completion checks are wrapped in a single
/// `catch_unwind` boundary — the closure and the "did we consume every
/// draw?" verdict share the same unwind scope, so there is no separate
/// terminator method whose behavior would depend on unrelated source
/// state.
///
/// Because [`AssertUnwindSafe`] does not roll back external state,
/// anything the closure mutated outside `TestCaseContext` before hitting a replay
/// mismatch stays mutated after this method returns. `TestCaseContext` external
/// state is not covered by the replay contract.
#[cfg(test)]
pub(crate) struct ReplaySession {
    sequence: ChoiceSequence,
}

#[cfg(test)]
impl ReplaySession {
    pub(crate) fn new(sequence: ChoiceSequence) -> Self {
        Self { sequence }
    }

    pub(crate) fn run<T, F>(self, f: F) -> Result<T, ReplayError>
    where
        F: FnOnce(&mut TestCaseContext) -> T,
    {
        let total_spans = self.sequence.spans.len();
        let mut ctx = TestCaseContext {
            source: RandomSource::Replay {
                sequence: self.sequence,
                next_draw: 0,
                next_span: 0,
                current_parent: None,
                error: None,
                explore: None,
                consumed: 0,
                pending_choice: None,
            },
            generated: Vec::new(),
            dedup: DedupState::default(),
            rejection: None,
            inside_runner: false,
            total_samples: 0,
            feedback: FeedbackState::Disabled,
        };
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut ctx)));
        let (sequence, next_draw, next_span, error) = match ctx.source {
            RandomSource::Replay {
                sequence,
                next_draw,
                next_span,
                error,
                ..
            } => (sequence, next_draw, next_span, error),
            _ => unreachable!("ReplaySession constructs Replay variant"),
        };

        // A stored replay error wins over both `Ok` (user caught our
        // marker and returned normally) and any `Err(payload)` — but a
        // non-marker user panic must still resume its own unwind.
        if let Some(err) = error {
            if let Err(payload) = outcome
                && !is_replay_abort(&*payload)
            {
                std::panic::resume_unwind(payload);
            }
            return Err(err);
        }

        match outcome {
            Ok(value) => {
                if next_draw < sequence.draws.len() {
                    Err(ReplayError::LeftoverDraws {
                        unused: sequence.draws.len() - next_draw,
                    })
                } else if next_span < total_spans {
                    Err(ReplayError::LeftoverSpans {
                        unused: total_spans - next_span,
                    })
                } else {
                    Ok(value)
                }
            }
            Err(payload) => {
                if is_replay_abort(&*payload) {
                    // Marker without stored error shouldn't happen —
                    // fill() sets error before resuming unwind. Report
                    // an exhausted sequence rather than silently dropping.
                    Err(ReplayError::SequenceExhausted { requested: 0 })
                } else {
                    std::panic::resume_unwind(payload)
                }
            }
        }
    }
}

/// Returns whether an unwind payload is the private
/// [`IterationRejected`] marker sent by [`TestCaseContext::reject_case`]. Used by
/// [`Runner::run`](crate::Runner::run) to distinguish rejection from
/// user panics.
pub(crate) fn is_iteration_rejected(payload: &(dyn Any + Send)) -> bool {
    payload.is::<IterationRejected>()
}

#[cfg(test)]
fn is_replay_abort(payload: &(dyn Any + Send)) -> bool {
    payload.is::<ReplayAbort>()
}

/// Compare two `Location`s by their surface fields.
///
/// We don't rely on pointer equality even though `Location::caller()`
/// typically returns the same static instance for a given call site —
/// value comparison is robust to any compiler dedup differences.
fn same_location(a: &Location<'_>, b: &Location<'_>) -> bool {
    a.file() == b.file() && a.line() == b.line() && a.column() == b.column()
}

/// SplitMix64 — a small-state PRNG used only for expanding the user seed
/// into the four 64-bit words of the `xoshiro256**` initial state.
///
/// Interposing SplitMix64 between the user seed and the xoshiro256**
/// state prevents low-quality seeds (0, small integers) from producing a
/// low-quality initial state.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Seed determinism (fill-based; next_u64 is no longer on TestCaseContext) ===

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = TestCaseContext::new(0xDEAD_BEEF);
        let mut b = TestCaseContext::new(0xDEAD_BEEF);
        for _ in 0..256 {
            let mut buf_a = [0u8; 8];
            let mut buf_b = [0u8; 8];
            a.fill(&mut buf_a);
            b.fill(&mut buf_b);
            assert_eq!(buf_a, buf_b);
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = TestCaseContext::new(1);
        let mut b = TestCaseContext::new(2);
        let mut buf_a = [0u8; 8];
        let mut buf_b = [0u8; 8];
        a.fill(&mut buf_a);
        b.fill(&mut buf_b);
        assert_ne!(buf_a, buf_b);
    }

    #[test]
    fn seed_zero_produces_nonzero_output() {
        let mut ctx = TestCaseContext::new(0);
        let mut buf = [0u8; 8];
        ctx.fill(&mut buf);
        assert_ne!(buf, [0u8; 8]);
    }

    #[test]
    fn fill_matches_le_bytes_of_xoshiro_step() {
        // `TestCaseContext::fill` must emit the little-endian bytes of successive
        // xoshiro256** `next_u64()` outputs, in order.
        let mut ctx = TestCaseContext::new(42);
        let mut xs = XoshiroState::from_seed(42);
        let mut buf = [0u8; 24];
        ctx.fill(&mut buf);
        for chunk in buf.chunks_exact(8) {
            assert_eq!(chunk, &xs.next_u64().to_le_bytes());
        }
    }

    #[test]
    fn fill_is_deterministic_for_non_multiple_of_eight() {
        let mut a = TestCaseContext::new(7);
        let mut b = TestCaseContext::new(7);
        let mut buf_a = [0u8; 5];
        let mut buf_b = [0u8; 5];
        a.fill(&mut buf_a);
        b.fill(&mut buf_b);
        assert_eq!(buf_a, buf_b);
    }

    #[test]
    fn fill_empty_buffer_does_not_advance() {
        // Filling an empty slice must not consume any PRNG state, so
        // the next non-empty fill must match a fresh TestCaseContext's first fill.
        let mut ctx = TestCaseContext::new(1);
        let empty: &mut [u8] = &mut [];
        ctx.fill(empty);
        let mut fresh = TestCaseContext::new(1);
        let mut buf_after = [0u8; 8];
        let mut buf_fresh = [0u8; 8];
        ctx.fill(&mut buf_after);
        fresh.fill(&mut buf_fresh);
        assert_eq!(buf_after, buf_fresh);
    }

    // === Bit-exact fill regression (byte-stream lock) ===

    #[test]
    fn fill_bit_exact_multiple_of_eight() {
        let mut ctx = TestCaseContext::new(0xDEAD_BEEF);
        let mut buf = [0u8; 16];
        ctx.fill(&mut buf);
        let mut fresh = TestCaseContext::new(0xDEAD_BEEF);
        let mut expected = [0u8; 16];
        fresh.fill(&mut expected);
        assert_eq!(buf, expected);
        assert_eq!(buf, expected_bytes(0xDEAD_BEEF, 16).as_slice(),);
    }

    #[test]
    fn fill_bit_exact_non_multiple_of_eight_tail() {
        let mut ctx = TestCaseContext::new(0xDEAD_BEEF);
        let mut buf = [0u8; 5];
        ctx.fill(&mut buf);
        assert_eq!(buf, expected_bytes(0xDEAD_BEEF, 5).as_slice());
    }

    #[test]
    fn fill_bit_exact_empty_slice_leaves_state_untouched() {
        let mut ctx = TestCaseContext::new(0xDEAD_BEEF);
        let empty: &mut [u8] = &mut [];
        ctx.fill(empty);
        let mut buf = [0u8; 8];
        ctx.fill(&mut buf);
        assert_eq!(buf, expected_bytes(0xDEAD_BEEF, 8).as_slice());
    }

    fn expected_bytes(seed: u64, n: usize) -> Vec<u8> {
        let mut xs = XoshiroState::from_seed(seed);
        let mut out = vec![0u8; n];
        xs.fill(&mut out);
        out
    }

    // === XoshiroState direct tests ===

    #[test]
    fn xoshiro_state_from_seed_is_deterministic() {
        let mut a = XoshiroState::from_seed(123);
        let mut b = XoshiroState::from_seed(123);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn xoshiro_state_fill_matches_next_u64() {
        let mut xs_fill = XoshiroState::from_seed(9);
        let mut xs_step = XoshiroState::from_seed(9);
        let mut buf = [0u8; 24];
        xs_fill.fill(&mut buf);
        for chunk in buf.chunks_exact(8) {
            assert_eq!(chunk, &xs_step.next_u64().to_le_bytes());
        }
    }

    // === Recording / Replay ===

    #[test]
    fn recording_session_captures_one_draw_per_non_empty_fill() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let mut b = [0u8; 3];
            ctx.fill(&mut b);
            let empty: &mut [u8] = &mut [];
            ctx.fill(empty);
            let mut c = [0u8; 8];
            ctx.fill(&mut c);
        });
        assert_eq!(seq.draws().len(), 3);
        assert_eq!(seq.draws()[0].len(), 4);
        assert_eq!(seq.draws()[1].len(), 3);
        assert_eq!(seq.draws()[2].len(), 8);
    }

    #[test]
    fn replay_reproduces_recorded_bytes() {
        let ((), seq) = RecordingSession::new(0xFEED).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let mut b = [0u8; 8];
            ctx.fill(&mut b);
        });

        let recorded_a = seq.draws()[0].clone();
        let recorded_b = seq.draws()[1].clone();

        let result = ReplaySession::new(seq)
            .run(|ctx| {
                let mut a = [0u8; 4];
                ctx.fill(&mut a);
                let mut b = [0u8; 8];
                ctx.fill(&mut b);
                (a.to_vec(), b.to_vec())
            })
            .expect("replay of same shape must succeed");
        assert_eq!(result.0, recorded_a);
        assert_eq!(result.1, recorded_b);
    }

    #[test]
    fn replay_reports_sequence_exhausted() {
        let (_, seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
        });
        assert!(matches!(
            result,
            Err(ReplayError::SequenceExhausted { requested: 4 })
        ));
    }

    #[test]
    fn replay_reports_draw_length_mismatch() {
        let (_, seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let mut a = [0u8; 8];
            ctx.fill(&mut a);
        });
        assert!(matches!(
            result,
            Err(ReplayError::DrawLengthMismatch {
                expected: 4,
                actual: 8,
            })
        ));
    }

    #[test]
    fn replay_reports_leftover_draws() {
        let (_, seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
        });
        assert!(matches!(
            result,
            Err(ReplayError::LeftoverDraws { unused: 1 })
        ));
    }

    #[test]
    fn replay_error_persists_when_user_catches_marker() {
        let (_, seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let mut a = [0u8; 4];
                ctx.fill(&mut a);
                let mut b = [0u8; 4];
                ctx.fill(&mut b);
            }));
            42u32
        });
        assert!(matches!(
            result,
            Err(ReplayError::SequenceExhausted { requested: 4 })
        ));
    }

    #[test]
    fn replay_reraises_non_marker_user_panic() {
        let (_, seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
        });
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = ReplaySession::new(seq).run(|_ctx| panic!("user panic"));
        }));
        let payload = outcome.expect_err("user panic must escape replay");
        let msg = payload
            .downcast_ref::<&'static str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("");
        assert!(msg.contains("user panic"), "unexpected payload: {msg:?}");
    }

    #[test]
    fn prng_source_allocates_no_choice_sequence() {
        let ctx = TestCaseContext::new(1);
        assert!(matches!(ctx.source, RandomSource::Prng(_)));
    }

    #[test]
    fn recording_and_prng_produce_same_bytes_for_same_seed() {
        let mut prng = TestCaseContext::new(0x00C0_FFEE);
        let mut expected = [0u8; 32];
        prng.fill(&mut expected);

        let ((), seq) = RecordingSession::new(0x00C0_FFEE).run(|ctx| {
            let mut buf = [0u8; 32];
            ctx.fill(&mut buf);
        });
        assert_eq!(seq.draws().len(), 1);
        assert_eq!(seq.draws()[0], expected);
    }

    // === reject_case Runner-only guard ===

    #[test]
    #[should_panic(expected = "Runner::run")]
    fn reject_case_outside_runner_panics_with_helpful_message() {
        let mut ctx = TestCaseContext::new(1);
        ctx.reject_case();
    }

    // === Attempt span recording + replay ===

    #[test]
    fn recording_captures_flat_attempt_spans() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            // Two top-level attempts, each of which does one 4-byte fill.
            let id = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            ctx.end_attempt(id, AttemptVerdict::Rejected);

            let id = ctx.begin_attempt();
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
            ctx.end_attempt(id, AttemptVerdict::Accepted);
        });
        assert_eq!(seq.spans().len(), 2);
        assert_eq!(seq.spans()[0].parent, None);
        assert_eq!(seq.spans()[0].start_draw, 0);
        assert_eq!(seq.spans()[0].end_draw, 1);
        assert_eq!(seq.spans()[0].verdict, AttemptVerdict::Rejected);
        assert_eq!(seq.spans()[1].parent, None);
        assert_eq!(seq.spans()[1].start_draw, 1);
        assert_eq!(seq.spans()[1].end_draw, 2);
        assert_eq!(seq.spans()[1].verdict, AttemptVerdict::Accepted);
    }

    #[test]
    fn recording_captures_nested_attempt_spans() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let outer = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let inner = ctx.begin_attempt();
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
            ctx.end_attempt(inner, AttemptVerdict::Rejected);
            let inner2 = ctx.begin_attempt();
            let mut c = [0u8; 4];
            ctx.fill(&mut c);
            ctx.end_attempt(inner2, AttemptVerdict::Accepted);
            ctx.end_attempt(outer, AttemptVerdict::Accepted);
        });
        assert_eq!(seq.spans().len(), 3);
        assert_eq!(seq.spans()[0].parent, None);
        assert_eq!(seq.spans()[1].parent, Some(0));
        assert_eq!(seq.spans()[2].parent, Some(0));
        assert_eq!(seq.spans()[0].start_draw, 0);
        assert_eq!(seq.spans()[0].end_draw, 3);
        assert_eq!(seq.spans()[1].start_draw, 1);
        assert_eq!(seq.spans()[1].end_draw, 2);
        assert_eq!(seq.spans()[2].start_draw, 2);
        assert_eq!(seq.spans()[2].end_draw, 3);
    }

    #[test]
    fn replay_reproduces_nested_attempt_spans() {
        let ((), seq) = RecordingSession::new(9).run(|ctx| {
            let outer = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let inner = ctx.begin_attempt();
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
            ctx.end_attempt(inner, AttemptVerdict::Rejected);
            ctx.end_attempt(outer, AttemptVerdict::Accepted);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let outer = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let inner = ctx.begin_attempt();
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
            ctx.end_attempt(inner, AttemptVerdict::Rejected);
            ctx.end_attempt(outer, AttemptVerdict::Accepted);
        });
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn replay_flags_verdict_mismatch() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let id = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            ctx.end_attempt(id, AttemptVerdict::Accepted);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let id = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            ctx.end_attempt(id, AttemptVerdict::Rejected);
        });
        assert!(matches!(
            result,
            Err(ReplayError::SpanMismatch {
                at_span: 0,
                reason: SpanMismatchReason::VerdictMismatch {
                    expected: AttemptVerdict::Accepted,
                    actual: AttemptVerdict::Rejected,
                }
            })
        ));
    }

    #[test]
    fn replay_flags_span_sequence_exhausted() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let id = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            ctx.end_attempt(id, AttemptVerdict::Accepted);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let id = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            ctx.end_attempt(id, AttemptVerdict::Accepted);
            let extra = ctx.begin_attempt();
            ctx.end_attempt(extra, AttemptVerdict::Accepted);
        });
        assert!(matches!(
            result,
            Err(ReplayError::SpanMismatch {
                at_span: 1,
                reason: SpanMismatchReason::SequenceExhausted,
            })
        ));
    }

    #[test]
    fn replay_flags_leftover_spans() {
        // Recording had a span wrapping one draw; replay consumed the
        // draw directly without a matching begin/end, so all draws are
        // used but one span is unread.
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let id = ctx.begin_attempt();
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            ctx.end_attempt(id, AttemptVerdict::Accepted);
        });
        let result = ReplaySession::new(seq).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
        });
        assert!(matches!(
            result,
            Err(ReplayError::LeftoverSpans { unused: 1 })
        ));
    }

    #[test]
    fn zero_draw_attempts_are_recorded_as_empty_range() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let id = ctx.begin_attempt();
            ctx.end_attempt(id, AttemptVerdict::Rejected);
            let id = ctx.begin_attempt();
            ctx.end_attempt(id, AttemptVerdict::Accepted);
        });
        assert_eq!(seq.spans().len(), 2);
        assert_eq!(seq.spans()[0].start_draw, seq.spans()[0].end_draw);
        assert_eq!(seq.spans()[1].start_draw, seq.spans()[1].end_draw);
        assert_eq!(seq.draws().len(), 0);
    }
}

#[cfg(test)]
mod targeted_tests {
    use super::*;

    // === maximize / feedback state ===

    #[test]
    fn maximize_keeps_maximum_within_case() {
        let mut ctx = TestCaseContext::new(1);
        ctx.enable_targeted();
        ctx.maximize(1.0);
        ctx.maximize(3.0);
        ctx.maximize(2.0);
        match ctx.take_feedback() {
            FeedbackState::Targeted { max_score } => {
                assert_eq!(max_score, ScalarFeedback::Valid(3.0));
            }
            other => panic!("expected targeted feedback, got {other:?}"),
        }
    }

    #[test]
    fn invalid_report_wins_over_earlier_valid_scores() {
        let mut ctx = TestCaseContext::new(1);
        ctx.enable_targeted();
        ctx.maximize(5.0);
        ctx.maximize(f64::NAN);
        match ctx.take_feedback() {
            FeedbackState::Targeted { max_score } => {
                assert_eq!(max_score, ScalarFeedback::Invalid);
            }
            other => panic!("expected targeted feedback, got {other:?}"),
        }
    }

    #[test]
    fn invalid_report_survives_later_valid_scores() {
        let mut ctx = TestCaseContext::new(1);
        ctx.enable_targeted();
        ctx.maximize(f64::NAN);
        ctx.maximize(5.0);
        match ctx.take_feedback() {
            FeedbackState::Targeted { max_score } => {
                assert_eq!(max_score, ScalarFeedback::Invalid);
            }
            other => panic!("expected targeted feedback, got {other:?}"),
        }
    }

    #[test]
    fn maximize_is_noop_when_disabled() {
        let mut ctx = TestCaseContext::new(1);
        ctx.maximize(f64::NAN);
        ctx.maximize(1.0);
        assert!(matches!(ctx.take_feedback(), FeedbackState::Disabled));
    }

    #[test]
    fn take_feedback_resets_to_disabled() {
        let mut ctx = TestCaseContext::new(1);
        ctx.enable_targeted();
        ctx.maximize(1.0);
        let _ = ctx.take_feedback();
        assert!(matches!(ctx.feedback, FeedbackState::Disabled));
    }

    // === choice metadata recording ===

    #[test]
    fn recording_tags_bounded_choice_and_integer_draws() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let _ = crate::sample_usize_in(ctx, 0..10);
            let _ = crate::sample_choice(ctx, &[1, 2, 3, 4]);
            let _ = crate::sample_u32(ctx);
        });
        let metas = seq.metas();
        assert!(matches!(metas[0], ChoiceMeta::Bounded { bound: 10 }));
        assert!(matches!(metas[1], ChoiceMeta::Choice { len: 4 }));
        assert!(matches!(metas[2], ChoiceMeta::Integer));
    }

    // === exploratory replay ===

    #[test]
    fn exploring_replays_recorded_draws_then_generates() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let mut buf = [0u8; 4];
            ctx.fill(&mut buf);
        });
        let recorded = seq.draws()[0].clone();
        assert_eq!(recorded.len(), 4);
        let mut ctx = TestCaseContext::exploring(seq, 42);
        let mut buf = [0u8; 4];
        ctx.fill(&mut buf);
        assert_eq!(
            buf,
            recorded.as_slice(),
            "the first draw must replay the recorded bytes"
        );
        // Draws beyond the recorded sequence are generated from the
        // explore PRNG and appended to the carried sequence.
        ctx.fill(&mut buf);
        let seq = ctx.take_sequence().expect("exploring must record draws");
        assert_eq!(seq.draws().len(), 2, "the generated draw must be recorded");
    }

    #[test]
    fn exploring_replays_mutated_draws() {
        let ((), mut seq) = RecordingSession::new(1).run(|ctx| {
            let mut buf = [0u8; 8];
            ctx.fill(&mut buf);
        });
        seq.draws_mut()[0][..8].copy_from_slice(&7u64.to_le_bytes());
        let mut ctx = TestCaseContext::exploring(seq, 42);
        let mut buf = [0u8; 8];
        ctx.fill(&mut buf);
        assert_eq!(
            u64::from_le_bytes(buf),
            7,
            "a mutated draw must be replayed with its new value"
        );
    }

    #[test]
    fn take_sequence_works_in_exploring_mode() {
        let mut ctx = TestCaseContext::exploring(ChoiceSequence::default(), 1);
        let mut buf = [0u8; 4];
        ctx.fill(&mut buf);
        let seq = ctx
            .take_sequence()
            .expect("exploring must yield a sequence");
        assert_eq!(seq.draws().len(), 1);
    }

    #[test]
    fn exploring_aborts_at_choice_cap() {
        let mut ctx = TestCaseContext::exploring(ChoiceSequence::default(), 1);
        let mut buf = [0u8; 4];
        for _ in 0..MAX_CHOICES_PER_CASE {
            ctx.fill(&mut buf);
        }
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            ctx.fill(&mut buf);
        }));
        assert!(outcome.is_err(), "drawing past the cap must unwind");
        assert!(
            ctx.take_rejection().is_some(),
            "the cap abort must be recorded as a rejection"
        );
    }

    #[test]
    fn exploring_aborts_at_cap_when_replacing_dead_draws() {
        // The width-mismatch replacement path shares the generated-
        // draw cap with the append path: repeatedly drawing a
        // different width than the recording must also reject at the
        // cap.
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let mut buf = [0u8; 4];
            ctx.fill(&mut buf);
        });
        let mut ctx = TestCaseContext::exploring(seq, 1);
        let mut buf = [0u8; 8];
        for _ in 0..MAX_CHOICES_PER_CASE {
            ctx.fill(&mut buf);
        }
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            ctx.fill(&mut buf);
        }));
        assert!(outcome.is_err(), "drawing past the cap must unwind");
        assert!(
            ctx.take_rejection().is_some(),
            "the cap abort must be recorded as a rejection"
        );
    }
}

#[cfg(test)]
mod regression_tests {
    use crate::rng::{ChoiceMeta, RecordingSession, TestCaseContext};

    #[test]
    fn exploring_adopts_new_meta_on_replay() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let mut buf = [0u8; 8];
            ctx.fill(&mut buf);
        });
        let recorded_value = seq.draws()[0].clone();
        let mut ctx = TestCaseContext::exploring(seq, 42);
        let mut buf = [0u8; 8];
        // Mimic a primitive whose constraint changed at the same draw
        // position (mutated control flow): declare the new domain, then
        // draw (replayed). The recorded metadata must be replaced by
        // the new declaration and the value regenerated...
        ctx.set_next_choice_meta(ChoiceMeta::Bounded { bound: 100 });
        ctx.fill(&mut buf);
        // ...and the declaration must not leak into the next generated
        // draw, which stays Raw.
        ctx.fill(&mut buf);
        let seq = ctx.take_sequence().expect("sequence");
        assert!(
            matches!(seq.metas()[0], ChoiceMeta::Bounded { bound: 100 }),
            "replayed draw must adopt the new declared domain: {:?}",
            seq.metas()[0]
        );
        assert_ne!(
            seq.draws()[0],
            recorded_value,
            "the value at a changed constraint must be regenerated"
        );
        assert!(
            matches!(seq.metas()[1], ChoiceMeta::Raw),
            "stale meta must not leak into a generated draw: {:?}",
            seq.metas()[1]
        );
    }

    #[test]
    fn exploring_replaces_dead_draws_in_place() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let mut buf = [0u8; 4];
            ctx.fill(&mut buf);
        });
        let mut ctx = TestCaseContext::exploring(seq, 42);
        let mut buf = [0u8; 8];
        // The mutated control flow asks for a different width than the
        // recorded draw: the dead draw is replaced in place, keeping
        // the sequence bounded.
        ctx.fill(&mut buf);
        let seq = ctx.take_sequence().expect("sequence");
        assert_eq!(
            seq.draws().len(),
            1,
            "dead draw must be replaced, not appended"
        );
        assert_eq!(seq.draws()[0].len(), 8);
    }

    #[test]
    fn exploring_replays_long_records_without_hitting_cap() {
        // The cap limits *generated* draws only; replayed recorded
        // draws are bounded by the recorded length.
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            for _ in 0..5000 {
                let mut buf = [0u8; 4];
                ctx.fill(&mut buf);
            }
        });
        let mut ctx = TestCaseContext::exploring(seq, 42);
        let mut buf = [0u8; 4];
        for _ in 0..5000 {
            ctx.fill(&mut buf);
        }
        // A generated draw after a long replay must still succeed.
        ctx.fill(&mut buf);
        let seq = ctx.take_sequence().expect("sequence");
        assert_eq!(seq.draws().len(), 5001);
    }
}

#[cfg(test)]
mod suffix_tests {
    use crate::rng::{RecordingSession, TestCaseContext};

    #[test]
    fn take_sequence_discards_unconsumed_suffix() {
        let ((), seq) = RecordingSession::new(1).run(|ctx| {
            let mut a = [0u8; 4];
            ctx.fill(&mut a);
            let mut b = [0u8; 4];
            ctx.fill(&mut b);
            let mut c = [0u8; 4];
            ctx.fill(&mut c);
        });
        assert_eq!(seq.draws().len(), 3);
        let mut ctx = TestCaseContext::exploring(seq, 42);
        // Only the first recorded draw is consumed; the rest is the
        // unconsumed suffix and must be discarded on take.
        let mut buf = [0u8; 4];
        ctx.fill(&mut buf);
        let seq = ctx.take_sequence().expect("sequence");
        assert_eq!(seq.draws().len(), 1, "unconsumed suffix must be discarded");
    }
}
