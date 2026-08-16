//! Entropy source and generated-value trace behind [`TestCaseContext`].

use std::any::Any;
use std::collections::VecDeque;
use std::panic::Location;

/// How many first values at the same location are kept verbatim in the
/// trace before elision starts.
const DEDUP_HEAD: usize = 8;

/// How many trailing values at the same location are kept in a rolling
/// buffer once the head is full.
const DEDUP_TAIL: usize = 8;

/// Per-case state threaded through every generator and property closure.
///
/// A `TestCaseContext` carries everything a case needs:
///
/// * **Entropy.** All random bytes come from an internal `xoshiro256**`
///   state seeded from a caller-supplied `u64` through `SplitMix64`.
///   noprop never draws entropy from the OS or the system clock: the
///   seed must always be provided by the caller, which makes every
///   property test exactly reproducible from its seed.
/// * **Trace.** Every `noprop::sample_*` call records the produced
///   value and its call site into the context; the trace is surfaced on
///   failure via [`RunError::generated`](crate::RunError::generated).
///   Raw PRNG state access is deliberately hidden so users cannot
///   accidentally bypass that trace.
/// * **Rejection.** [`TestCaseContext::reject_case`] unwinds out of the
///   property closure to reject the current iteration. Prefer the
///   bounded-retry helper
///   [`sample_with_rejection`](crate::sample_with_rejection) when the
///   exit is expressible as "retry this sample".
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
    state: XoshiroState,
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
    /// observed on this context (never reset between cases). The runner
    /// reuses one context across the whole run and reads this directly
    /// for [`Stats::total_samples`](crate::Stats::total_samples).
    /// Counted per `record_generated` call - most primitives record
    /// exactly once per invocation, but composite helpers such as
    /// `sample_with_boundaries` record twice (the ratio's `bool` plus
    /// the chosen value). Dedup / elision happens after the counter is
    /// incremented, so folded runs are still fully counted.
    total_samples: usize,
}

/// Private `xoshiro256**` state and step function.
struct XoshiroState {
    state: [u64; 4],
}

impl XoshiroState {
    fn from_seed(seed: u64) -> Self {
        let mut sm = SplitMix64 { state: seed };
        Self {
            state: [sm.next(), sm.next(), sm.next(), sm.next()],
        }
    }

    /// Advance the PRNG by one step and return the next 64 random bits.
    fn next_u64(&mut self) -> u64 {
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

    fn fill(&mut self, dst: &mut [u8]) {
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
}

/// Private control-flow marker used by [`TestCaseContext::reject_case`] to unwind
/// out of the property closure. [`Runner::run`](crate::Runner::run)
/// catches it and, in combination with the `rejection` state saved on
/// the [`TestCaseContext`], treats the iteration as rejected. Never sent from a
/// context that lacks the catching boundary — `reject_case` checks
/// `TestCaseContext::inside_runner` first.
pub(crate) struct IterationRejected;

/// Diagnostic state saved by [`TestCaseContext::reject_case`] before it unwinds.
/// Runner reads this after `catch_unwind` and reports it on
/// [`TooManyRejections`](crate::RunError) failures.
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
///   [`RunError::generated`](crate::RunError::generated) for the elision
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

impl TestCaseContext {
    /// Create a new [`TestCaseContext`] from a 64-bit seed.
    ///
    /// The seed is expanded to the 256-bit internal state through
    /// SplitMix64. Passing the same seed twice always produces the same
    /// output stream.
    pub fn new(seed: u64) -> Self {
        Self {
            state: XoshiroState::from_seed(seed),
            generated: Vec::new(),
            dedup: DedupState::default(),
            rejection: None,
            inside_runner: false,
            total_samples: 0,
        }
    }

    /// Reject the current iteration and unwind out of the property
    /// closure. Only valid inside [`Runner::run`](crate::Runner::run);
    /// calling this from a `TestCaseContext` constructed outside a runner panics
    /// with a Runner-only message.
    ///
    /// Prefer
    /// [`sample_with_rejection`](crate::sample_with_rejection) as the
    /// high-level bounded-retry helper. Reach for `reject_case`
    /// directly only when the whole case turns out to be an
    /// unsuitable candidate *after* sampling has finished and the
    /// helper cannot express the exit.
    ///
    /// The runner discards the current case's generated-value
    /// trace and tries the next case. Rejected cases are not
    /// counted toward
    /// `cases`; rejected +
    /// accepted attempts together are bounded by an internal global
    /// limit so a generator that always rejects still terminates.
    #[track_caller]
    pub fn reject_case(&mut self) -> ! {
        if !self.inside_runner {
            panic!(
                "noprop::TestCaseContext::reject_case can only be called from inside a Runner::run \
                 property closure. Constructing a TestCaseContext directly via \
                 TestCaseContext::new does not create a Runner boundary."
            );
        }
        let location = Location::caller();
        self.rejection = Some(RejectionState { location });
        std::panic::resume_unwind(Box::new(IterationRejected));
    }

    /// Fill `dst` with random bytes. An empty slice consumes no RNG
    /// state.
    ///
    /// This is the single entropy boundary of [`TestCaseContext`].
    ///
    /// `pub(crate)` — see the [`TestCaseContext`] type-level docs for why.
    pub(crate) fn fill(&mut self, dst: &mut [u8]) {
        if dst.is_empty() {
            return;
        }
        self.state.fill(dst);
    }

    /// Consume and return any pending rejection state saved by
    /// [`TestCaseContext::reject_case`]. Called by
    /// [`Runner::run`](crate::Runner::run) after each case boundary so
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
    /// [`Runner::run`](crate::Runner::run) immediately after
    /// constructing its `TestCaseContext`.
    pub(crate) fn set_inside_runner(&mut self) {
        self.inside_runner = true;
    }

    /// `true` when this context was constructed inside a
    /// [`Runner::run`](crate::Runner::run) invocation. Callers that
    /// would trigger an iteration rejection (e.g. `sample_with_rejection`
    /// on exhaustion) can query this to tailor their panic message
    /// rather than propagating the generic `reject_case` "not inside a
    /// Runner" panic.
    pub(crate) fn is_inside_runner(&self) -> bool {
        self.inside_runner
    }

    /// Total number of top-level `sample_*` invocations observed on this
    /// context. Consumed by [`Runner::run`](crate::Runner::run) for
    /// [`Stats::total_samples`](crate::Stats::total_samples).
    pub(crate) fn total_samples(&self) -> usize {
        self.total_samples
    }
}

/// Returns whether an unwind payload is the private
/// [`IterationRejected`] marker sent by [`TestCaseContext::reject_case`]. Used by
/// [`Runner::run`](crate::Runner::run) to distinguish rejection from
/// user panics.
pub(crate) fn is_iteration_rejected(payload: &(dyn Any + Send)) -> bool {
    payload.is::<IterationRejected>()
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

    // === Seed determinism (via TestCaseContext::fill) ===

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

    // === reject_case Runner-only guard ===

    #[test]
    #[should_panic(expected = "Runner::run")]
    fn reject_case_outside_runner_panics_with_helpful_message() {
        let mut ctx = TestCaseContext::new(1);
        ctx.reject_case();
    }
}
