//! Non-cryptographic seedable PRNG (xoshiro256** with SplitMix64 seed expansion).

use std::collections::VecDeque;
use std::panic::Location;

/// How many first values at the same location are kept verbatim in the
/// trace before elision starts.
const DEDUP_HEAD: usize = 8;

/// How many trailing values at the same location are kept in a rolling
/// buffer once the head is full.
const DEDUP_TAIL: usize = 8;

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
/// The only public method is [`Rng::new`]; all byte/word production
/// happens through the `noprop::gen_*` free functions, which record the
/// generated values into an internal trace surfaced on failure. Raw
/// PRNG state access is deliberately hidden so users cannot accidentally
/// bypass that trace.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0xDEAD_BEEF);
/// let a = noprop::gen_u32(&mut rng);
/// let b = noprop::gen_u32(&mut rng);
/// assert_ne!(a, b);
/// ```
pub struct Rng {
    state: [u64; 4],
    generated: Vec<GeneratedValue>,
    dedup: DedupState,
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

impl Rng {
    /// Create a new [`Rng`] from a 64-bit seed.
    ///
    /// The seed is expanded to the 256-bit internal state through
    /// SplitMix64. Passing the same seed twice always produces the same
    /// output stream.
    pub fn new(seed: u64) -> Self {
        let mut sm = SplitMix64 { state: seed };
        Self {
            state: [sm.next(), sm.next(), sm.next(), sm.next()],
            generated: Vec::new(),
            dedup: DedupState::default(),
        }
    }

    /// Advance the PRNG by one step and return the next 64 random bits.
    ///
    /// This is the raw `xoshiro256**` step function. `pub(crate)`
    /// so that primitive generators can build on it without exposing
    /// a trace-bypassing entry point.
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

    /// Fill `dst` with random bytes. An empty slice consumes no RNG state.
    ///
    /// `pub(crate)` — see the [`Rng`] type-level docs for why.
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

    /// Record a generated value in this Rng's buffer. Called from every
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
    /// composite generator (e.g. a user-defined `gen_person` called
    /// inside a loop) also folds correctly.
    #[track_caller]
    pub(crate) fn record_generated<T: std::fmt::Debug + Clone + 'static>(
        &mut self,
        value: &T,
        location: &'static Location<'static>,
    ) {
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

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = Rng::new(0xDEAD_BEEF);
        let mut b = Rng::new(0xDEAD_BEEF);
        for _ in 0..256 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn seed_zero_produces_nonzero_output() {
        // SplitMix64 seeding side-steps the xoshiro256** all-zero trap.
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
    }

    #[test]
    fn fill_matches_le_bytes_of_next_u64() {
        let mut fill_rng = Rng::new(42);
        let mut ref_rng = Rng::new(42);

        let mut buf = [0u8; 24];
        fill_rng.fill(&mut buf);

        for chunk in buf.chunks_exact(8) {
            let expected = ref_rng.next_u64().to_le_bytes();
            assert_eq!(chunk, &expected);
        }
    }

    #[test]
    fn fill_is_deterministic_for_non_multiple_of_eight() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let mut buf_a = [0u8; 5];
        let mut buf_b = [0u8; 5];
        a.fill(&mut buf_a);
        b.fill(&mut buf_b);
        assert_eq!(buf_a, buf_b);
    }

    #[test]
    fn fill_empty_buffer_does_not_advance() {
        let mut rng = Rng::new(1);
        let mut buf: [u8; 0] = [];
        rng.fill(&mut buf);
        // Filling an empty slice must not consume any RNG state.
        let mut fresh = Rng::new(1);
        assert_eq!(rng.next_u64(), fresh.next_u64());
    }
}
