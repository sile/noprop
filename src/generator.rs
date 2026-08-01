//! Value generators.
//!
//! Every generator has the shape `fn sample_X(rng: &mut Rng) -> X`. User
//! code composes generators with plain Rust — closures, `match`, `for`,
//! iterators — so property tests read like ordinary Rust code, and user
//! generators (`fn sample_person(rng: &mut Rng) -> Person`) have the
//! same shape as the built-in ones.
//!
//! # Composing generators
//!
//! To build a generator whose output depends on another's, just call
//! them sequentially inside a plain function or closure:
//!
//! ```
//! use noprop::Rng;
//!
//! fn sample_bounded_vec(rng: &mut Rng) -> Vec<u32> {
//!     // Pick a length first, then a Vec of that length.
//!     let len = noprop::sample_usize_in(rng, 0..10);
//!     (0..len).map(|_| noprop::sample_u32(rng)).collect()
//! }
//!
//! let mut rng = Rng::new(0);
//! let _v: Vec<u32> = sample_bounded_vec(&mut rng);
//! ```
//!
//! For "one-of-N" branching between code paths, `match` on a small
//! random value produced by [`sample_usize_in`]:
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! let _x: u32 = match noprop::sample_usize_in(&mut rng, 0..3) {
//!     0 => 0,
//!     1 => noprop::sample_u32(&mut rng),
//!     _ => u32::MAX,
//! };
//! ```
//!
//! To weight the branches unevenly, use [`sample_weighted_index`] (or
//! [`sample_ratio`] for a two-way split):
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! // Pick branch 0 with weight 5, branch 1 with weight 3, branch 2 with weight 2.
//! let _x: u32 = match noprop::sample_weighted_index(&mut rng, &[5, 3, 2]) {
//!     0 => 0,
//!     1 => noprop::sample_u32(&mut rng),
//!     _ => u32::MAX,
//! };
//! ```
//!
//! To pick one value from a fixed list, use [`sample_choice`]:
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! let _n = noprop::sample_choice(&mut rng, &[1, 2, 3, 5, 8]);
//! let _digit = noprop::sample_choice(&mut rng, b"0123456789") as char;
//! ```
//!
//! For bounded retry (filter-style generation), combine a range iterator
//! with `.find()`:
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! let even: Option<u32> = (0..100)
//!     .map(|_| noprop::sample_u32(&mut rng))
//!     .find(|x| x % 2 == 0);
//! # assert!(even.is_some());
//! ```

use std::num::NonZero;
use std::ops::{Bound, RangeBounds};
use std::panic::Location;

use crate::Rng;

/// Read `N` bytes from `rng` without recording. Used by every primitive
/// so that composite generators (non-zero variants, `sample_char`,
/// floats, `sample_choice`) can consume randomness without producing
/// intermediate trace entries for the raw byte source.
fn raw_bytes<const N: usize>(rng: &mut Rng) -> [u8; N] {
    let mut buf = [0u8; N];
    rng.fill(&mut buf);
    buf
}

// === Bounded-domain sampler ===

/// Sample a uniform `u64` in `[0, n)` using rejection sampling.
///
/// Uses `u64` as a pointer-width-independent working domain so the same
/// draw pattern applies to every finite-domain selection primitive
/// (`sample_usize_in`, `sample_ratio`, `sample_weighted_index`,
/// `sample_choice`).
/// Draws are consumed from the RNG only via [`raw_bytes`], so rejected
/// attempts do not appear in the trace.
///
/// Panics in debug builds if `n == 0`. Callers must guarantee `n > 0`.
fn sample_below(rng: &mut Rng, n: u64) -> u64 {
    debug_assert!(n > 0, "sample_below: n must be non-zero");
    if n == 1 {
        return 0;
    }
    // We want to sample uniformly from [0, n) by drawing x from
    // [0, 2^64) and returning x % n. To avoid modulo bias we must only
    // accept draws that fall in a range whose size is a multiple of n.
    //
    // Let r = u64::MAX % n. Then 2^64 mod n = (r + 1) mod n:
    //   - If r == n - 1, n divides 2^64: every draw is unbiased.
    //   - Otherwise the unbiased zone has 2^64 - (r + 1) = u64::MAX - r
    //     values, i.e. accept iff x < u64::MAX - r.
    let r = u64::MAX % n;
    if r == n - 1 {
        return u64::from_le_bytes(raw_bytes(rng)) % n;
    }
    let bound = u64::MAX - r;
    loop {
        let x = u64::from_le_bytes(raw_bytes(rng));
        if x < bound {
            return x % n;
        }
    }
}

// === Selection helpers ===

/// Pick one element from `choices` uniformly at random.
///
/// This is the noprop counterpart to picking from a fixed list. Use it
/// when the alternatives are *values*; for branching between code paths
/// (calling different generators, taking different actions), use `match`
/// on [`sample_usize_in`] or [`sample_weighted_index`] instead — see the
/// module docstring.
///
/// # Panics
///
/// Panics if `choices` is empty.
///
/// # Determinism note
///
/// Uses the same rejection-sampling core as [`sample_usize_in`]. For
/// most slice lengths the rejection rate is negligible, but the number
/// of RNG bytes consumed by a call is not fixed, so the exact output
/// stream for a given seed depends on the slice length.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// // Explicit list of ints
/// let _n = noprop::sample_choice(&mut rng, &[1, 2, 3, 5, 8]);
/// // ASCII digit from a byte string literal
/// let _d = noprop::sample_choice(&mut rng, b"0123456789") as char;
/// // Non-ASCII via array literal
/// let _c = noprop::sample_choice(&mut rng, &['α', 'β', 'γ']);
/// ```
#[track_caller]
pub fn sample_choice<T: Clone + std::fmt::Debug + 'static>(rng: &mut Rng, choices: &[T]) -> T {
    assert!(!choices.is_empty(), "sample_choice: empty slice");
    let loc = Location::caller();
    let idx = sample_below(rng, choices.len() as u64) as usize;
    let v = choices[idx].clone();
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `usize` inside `range`.
///
/// Accepts any `RangeBounds<usize>` — `a..b`, `a..=b`, `..b`, `a..`,
/// `..`, and so on. The typical use is picking a collection length,
/// slice index, loop count, or small branch discriminator without
/// having to write `% N` (which is both easy to overflow at
/// `usize::MAX` and biased when the divisor does not evenly divide the
/// integer domain).
///
/// # Panics
///
/// Panics if `range` is empty (e.g. `5..5`, `5..=4`, `..0`, or an
/// excluded start of `usize::MAX`).
///
/// # Determinism note
///
/// Uses rejection sampling internally. Most calls accept the first
/// draw, but the exact byte count consumed depends on the range width,
/// so changing the range can shift subsequent RNG output for the same
/// seed. This is a deliberate correction over the earlier `sample_usize
/// % (max + 1)` recipe, which was both bias-prone and overflow-prone.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
///
/// let idx = noprop::sample_usize_in(&mut rng, 0..10);
/// assert!(idx < 10);
///
/// let day = noprop::sample_usize_in(&mut rng, 1..=31);
/// assert!((1..=31).contains(&day));
/// ```
#[track_caller]
pub fn sample_usize_in<R: RangeBounds<usize>>(rng: &mut Rng, range: R) -> usize {
    let loc = Location::caller();
    let lo = match range.start_bound() {
        Bound::Included(&s) => s,
        Bound::Excluded(&s) => s.checked_add(1).expect("sample_usize_in: empty range"),
        Bound::Unbounded => 0,
    };
    let hi = match range.end_bound() {
        Bound::Included(&e) => e,
        Bound::Excluded(&e) => e.checked_sub(1).expect("sample_usize_in: empty range"),
        Bound::Unbounded => usize::MAX,
    };
    assert!(lo <= hi, "sample_usize_in: empty range");
    let v = if lo == 0 && hi == usize::MAX {
        // Full pointer-width range: a raw byte draw is already unbiased,
        // and hi - lo + 1 would wrap.
        usize::from_le_bytes(raw_bytes(rng))
    } else {
        // hi - lo cannot overflow because hi >= lo, and (hi - lo) + 1
        // cannot overflow because we excluded the only case where
        // hi - lo == usize::MAX. Cast to u64 is safe on every Rust
        // target (usize width <= 64).
        let width = (hi - lo) as u64 + 1;
        lo + sample_below(rng, width) as usize
    };
    rng.record_generated(&v, loc);
    v
}

/// Return `true` with probability `numerator / denominator`.
///
/// The typical use is weighting a two-way branch by an exact rational
/// probability instead of a floating-point one, so that
/// e.g. `sample_ratio(rng, 1, 3)` is exactly one-in-three, not
/// `0.333…`-close.
///
/// # Panics
///
/// - Panics if `denominator == 0`.
/// - Panics if `numerator > denominator`.
///
/// # Determinism note
///
/// The degenerate cases `numerator == 0` (always `false`) and
/// `numerator == denominator` (always `true`) consume no RNG bytes, so
/// tuning a weight down to 0 or up to 100% does not shift subsequent
/// output. All other cases consume RNG bytes through the shared
/// rejection sampler, and the exact byte count depends on
/// `denominator`.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// // 1 in 3 chance of true.
/// let _b = noprop::sample_ratio(&mut rng, 1, 3);
/// // Always false; consumes no RNG.
/// assert!(!noprop::sample_ratio(&mut rng, 0, 5));
/// // Always true; consumes no RNG.
/// assert!(noprop::sample_ratio(&mut rng, 5, 5));
/// ```
#[track_caller]
pub fn sample_ratio(rng: &mut Rng, numerator: u32, denominator: u32) -> bool {
    let loc = Location::caller();
    assert!(
        denominator != 0,
        "sample_ratio: denominator must be non-zero"
    );
    assert!(
        numerator <= denominator,
        "sample_ratio: numerator ({numerator}) must be <= denominator ({denominator})"
    );
    let v = if numerator == 0 {
        false
    } else if numerator == denominator {
        true
    } else {
        sample_below(rng, denominator as u64) < numerator as u64
    };
    rng.record_generated(&v, loc);
    v
}

/// Pick an index from `weights` with probability proportional to each
/// weight.
///
/// The typical use is picking a branch or command variant with an
/// uneven distribution (e.g. 10× more `read` operations than `write`).
/// The chosen index is returned so the caller can `match` on it and
/// call the corresponding generator or action — the weight list carries
/// no values of its own.
///
/// # Panics
///
/// - Panics if `weights` is empty.
/// - Panics if every weight is `0` (no branch is selectable).
/// - Panics if the sum of weights overflows `u64` (in practice this
///   requires a `weights` slice that cannot fit in addressable memory).
///
/// # Determinism note
///
/// Uses the shared rejection sampler on the sum of weights, so the
/// exact number of RNG bytes consumed depends on that sum. Adding a
/// zero-weighted variant to the slice does change the sampler input
/// (the slice length grows) even though it never affects the returned
/// index distribution.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// // Roughly 50% branch 0, 30% branch 1, 20% branch 2.
/// let idx = noprop::sample_weighted_index(&mut rng, &[5, 3, 2]);
/// assert!(idx < 3);
/// ```
#[track_caller]
pub fn sample_weighted_index(rng: &mut Rng, weights: &[u32]) -> usize {
    let loc = Location::caller();
    assert!(!weights.is_empty(), "sample_weighted_index: empty weights");
    let mut total: u64 = 0;
    for &w in weights {
        total = total
            .checked_add(w as u64)
            .expect("sample_weighted_index: weight sum overflows u64");
    }
    assert!(total > 0, "sample_weighted_index: all weights are zero");
    let mut pick = sample_below(rng, total);
    let mut chosen = weights.len(); // sentinel; overwritten below
    for (i, &w) in weights.iter().enumerate() {
        let w = w as u64;
        if pick < w {
            chosen = i;
            break;
        }
        pick -= w;
    }
    // Every non-zero-weight index is reachable and pick < total, so the
    // loop must have hit the `pick < w` branch at least once.
    debug_assert!(chosen < weights.len());
    rng.record_generated(&chosen, loc);
    chosen
}

// === Byte generators ===

/// Uniformly-distributed fixed-size byte array.
///
/// The generic parameter `N` sets the array length at compile time, so
/// the whole result is stack-allocated and recorded as a single trace
/// entry (`[u8; N] = [...]`) rather than N separate `u8` entries.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// let key: [u8; 32] = noprop::sample_bytes(&mut rng);
/// assert_eq!(key.len(), 32);
/// ```
#[track_caller]
pub fn sample_bytes<const N: usize>(rng: &mut Rng) -> [u8; N] {
    let loc = Location::caller();
    let bytes = raw_bytes::<N>(rng);
    rng.record_generated(&bytes, loc);
    bytes
}

/// Uniformly-distributed `Vec<u8>` of length `len`.
///
/// Use this when the byte-buffer length is known only at runtime
/// (`sample_bytes_vec(rng, sample_usize_in(rng, 0..1024))`). The whole
/// buffer is recorded as a single trace entry.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// let bytes = noprop::sample_bytes_vec(&mut rng, 100);
/// assert_eq!(bytes.len(), 100);
/// ```
#[track_caller]
pub fn sample_bytes_vec(rng: &mut Rng, len: usize) -> Vec<u8> {
    let loc = Location::caller();
    let mut bytes = vec![0u8; len];
    rng.fill(&mut bytes);
    rng.record_generated(&bytes, loc);
    bytes
}

// === Boolean generator ===

/// Uniformly-distributed `bool`.
#[track_caller]
pub fn sample_bool(rng: &mut Rng) -> bool {
    let loc = Location::caller();
    // Consume one byte so this primitive shares the "read a fixed-size
    // byte slice" shape with the integer generators.
    let v = raw_bytes::<1>(rng)[0] & 1 != 0;
    rng.record_generated(&v, loc);
    v
}

// === Integer generators ===
//
// All primitives draw randomness through `Rng::fill` (LE bytes ->
// `from_le_bytes`) so that every primitive consumes a fixed-size byte
// slice from the RNG. This keeps every generator compatible with a
// future bytes-based shrink implementation that swaps the RNG for a
// byte reader.

/// Uniformly-distributed `u8`.
#[track_caller]
pub fn sample_u8(rng: &mut Rng) -> u8 {
    let loc = Location::caller();
    let v = raw_bytes::<1>(rng)[0];
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u16`.
#[track_caller]
pub fn sample_u16(rng: &mut Rng) -> u16 {
    let loc = Location::caller();
    let v = u16::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u32`.
#[track_caller]
pub fn sample_u32(rng: &mut Rng) -> u32 {
    let loc = Location::caller();
    let v = u32::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u64`.
#[track_caller]
pub fn sample_u64(rng: &mut Rng) -> u64 {
    let loc = Location::caller();
    let v = u64::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u128`.
#[track_caller]
pub fn sample_u128(rng: &mut Rng) -> u128 {
    let loc = Location::caller();
    let v = u128::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `usize`.
#[track_caller]
pub fn sample_usize(rng: &mut Rng) -> usize {
    let loc = Location::caller();
    let v = usize::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i8`.
#[track_caller]
pub fn sample_i8(rng: &mut Rng) -> i8 {
    let loc = Location::caller();
    let v = raw_bytes::<1>(rng)[0] as i8;
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i16`.
#[track_caller]
pub fn sample_i16(rng: &mut Rng) -> i16 {
    let loc = Location::caller();
    let v = i16::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i32`.
#[track_caller]
pub fn sample_i32(rng: &mut Rng) -> i32 {
    let loc = Location::caller();
    let v = i32::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i64`.
#[track_caller]
pub fn sample_i64(rng: &mut Rng) -> i64 {
    let loc = Location::caller();
    let v = i64::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i128`.
#[track_caller]
pub fn sample_i128(rng: &mut Rng) -> i128 {
    let loc = Location::caller();
    let v = i128::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `isize`.
#[track_caller]
pub fn sample_isize(rng: &mut Rng) -> isize {
    let loc = Location::caller();
    let v = isize::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

// === Non-zero integer generators ===
//
// Each `sample_non_zero_*` uses rejection sampling: read the underlying
// integer and retry on zero. P(zero) is at most 1/256 per attempt for
// every type below, so the 64-attempt bound is effectively unreachable
// (worst-case P(all zero) < (1/256)^64 ~ 10^-154 for u8; even smaller
// elsewhere). Intermediate rejected attempts are not recorded — only
// the final NonZero value is.

/// Uniformly-distributed non-zero `u8`.
#[track_caller]
pub fn sample_non_zero_u8(rng: &mut Rng) -> NonZero<u8> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(raw_bytes::<1>(rng)[0]) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_u8: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u16`.
#[track_caller]
pub fn sample_non_zero_u16(rng: &mut Rng) -> NonZero<u16> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u16::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_u16: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u32`.
#[track_caller]
pub fn sample_non_zero_u32(rng: &mut Rng) -> NonZero<u32> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u32::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_u32: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u64`.
#[track_caller]
pub fn sample_non_zero_u64(rng: &mut Rng) -> NonZero<u64> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u64::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_u64: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u128`.
#[track_caller]
pub fn sample_non_zero_u128(rng: &mut Rng) -> NonZero<u128> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u128::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_u128: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `usize`.
#[track_caller]
pub fn sample_non_zero_usize(rng: &mut Rng) -> NonZero<usize> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(usize::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_usize: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i8`.
#[track_caller]
pub fn sample_non_zero_i8(rng: &mut Rng) -> NonZero<i8> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(raw_bytes::<1>(rng)[0] as i8) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_i8: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i16`.
#[track_caller]
pub fn sample_non_zero_i16(rng: &mut Rng) -> NonZero<i16> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i16::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_i16: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i32`.
#[track_caller]
pub fn sample_non_zero_i32(rng: &mut Rng) -> NonZero<i32> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i32::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_i32: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i64`.
#[track_caller]
pub fn sample_non_zero_i64(rng: &mut Rng) -> NonZero<i64> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i64::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_i64: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i128`.
#[track_caller]
pub fn sample_non_zero_i128(rng: &mut Rng) -> NonZero<i128> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i128::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_i128: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `isize`.
#[track_caller]
pub fn sample_non_zero_isize(rng: &mut Rng) -> NonZero<isize> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(isize::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("sample_non_zero_isize: rejection sampling exhausted")
}

// === Character generators ===
//
// For character subsets beyond the ones below (alphanumeric, hexdigit,
// etc.), compose with `sample_choice` over a byte-string literal, for
// example:
//
//     let d = sample_choice(rng, b"0123456789") as char;
//     let a = sample_choice(rng, b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789") as char;

/// Uniformly-distributed `char` over the valid Unicode scalar values
/// (`0..=0x10FFFF`, excluding the surrogate range `0xD800..=0xDFFF`).
#[track_caller]
pub fn sample_char(rng: &mut Rng) -> char {
    let loc = Location::caller();
    // Rejection sampling on a 21-bit mask; expected rejection rate is
    // about 47%, so the 64-attempt bound is unreachable in practice
    // (P(all 64 fail) < 10^-20).
    for _ in 0..64 {
        let n = u32::from_le_bytes(raw_bytes(rng)) & 0x1F_FFFF;
        if let Some(c) = char::from_u32(n) {
            rng.record_generated(&c, loc);
            return c;
        }
    }
    panic!("sample_char: rejection sampling exhausted")
}

/// Uniformly-distributed ASCII `char` (`0x00..=0x7F`, including control
/// characters).
#[track_caller]
pub fn sample_ascii_char(rng: &mut Rng) -> char {
    let loc = Location::caller();
    let v = (raw_bytes::<1>(rng)[0] & 0x7F) as char;
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed printable ASCII `char` (`0x20..=0x7E`, space
/// through `~`, excluding control characters and DEL).
#[track_caller]
pub fn sample_ascii_printable_char(rng: &mut Rng) -> char {
    let loc = Location::caller();
    // 95 characters. Use u32 for negligible modulo bias
    // (2^32 mod 95 = 6, so bias factor is at most 1 + 1/45210182).
    let v = (0x20 + u32::from_le_bytes(raw_bytes(rng)) % 95) as u8 as char;
    rng.record_generated(&v, loc);
    v
}

// === Floating-point generators ===

/// Uniformly-distributed `f32` in `[min, max)`.
///
/// NaN and infinities are excluded from the output range. To include
/// them (or any specific special value), pick from a fixed set with
/// [`sample_choice`]:
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// let _x = noprop::sample_choice(
///     &mut rng,
///     &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0],
/// );
/// ```
///
/// For an arbitrary `f32` bit pattern (including NaN, infinities, and
/// subnormals):
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// let _x = f32::from_bits(noprop::sample_u32(&mut rng));
/// ```
///
/// # Panics
///
/// Panics if `min` or `max` is not finite, or if `min >= max`.
#[track_caller]
pub fn sample_f32(rng: &mut Rng, min: f32, max: f32) -> f32 {
    assert!(
        min.is_finite() && max.is_finite(),
        "sample_f32: min and max must be finite"
    );
    assert!(min < max, "sample_f32: min must be less than max");
    let loc = Location::caller();
    // Build a 24-bit uniform value in [0, 1): construct a float in
    // [1, 2) by injecting 23 random bits into the mantissa of a fixed
    // exponent, then subtract 1. This is bias-free (every representable
    // value in [0, 1) with 24-bit precision is equally likely).
    let bits = 0x3F80_0000 | (u32::from_le_bytes(raw_bytes(rng)) >> 9);
    let unit = f32::from_bits(bits) - 1.0;
    let v = min + (max - min) * unit;
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `f64` in `[min, max)`.
///
/// Same conventions as [`sample_f32`]: NaN and infinities are excluded from
/// the output. Use [`sample_choice`] to include specific special values, or
/// `f64::from_bits(sample_u64(rng))` for an arbitrary bit pattern.
///
/// # Panics
///
/// Panics if `min` or `max` is not finite, or if `min >= max`.
#[track_caller]
pub fn sample_f64(rng: &mut Rng, min: f64, max: f64) -> f64 {
    assert!(
        min.is_finite() && max.is_finite(),
        "sample_f64: min and max must be finite"
    );
    assert!(min < max, "sample_f64: min must be less than max");
    let loc = Location::caller();
    // Same construction as sample_f32 but with 53-bit precision.
    let bits = 0x3FF0_0000_0000_0000 | (u64::from_le_bytes(raw_bytes(rng)) >> 12);
    let unit = f64::from_bits(bits) - 1.0;
    let v = min + (max - min) * unit;
    rng.record_generated(&v, loc);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_are_deterministic() {
        let mut a = Rng::new(123);
        let mut b = Rng::new(123);
        assert_eq!(sample_bool(&mut a), sample_bool(&mut b));
        assert_eq!(sample_u8(&mut a), sample_u8(&mut b));
        assert_eq!(sample_u16(&mut a), sample_u16(&mut b));
        assert_eq!(sample_u32(&mut a), sample_u32(&mut b));
        assert_eq!(sample_u64(&mut a), sample_u64(&mut b));
        assert_eq!(sample_u128(&mut a), sample_u128(&mut b));
        assert_eq!(sample_usize(&mut a), sample_usize(&mut b));
        assert_eq!(sample_i8(&mut a), sample_i8(&mut b));
        assert_eq!(sample_i16(&mut a), sample_i16(&mut b));
        assert_eq!(sample_i32(&mut a), sample_i32(&mut b));
        assert_eq!(sample_i64(&mut a), sample_i64(&mut b));
        assert_eq!(sample_i128(&mut a), sample_i128(&mut b));
        assert_eq!(sample_isize(&mut a), sample_isize(&mut b));
    }

    #[test]
    fn bool_produces_both_values() {
        let mut rng = Rng::new(1);
        let (mut t, mut f) = (false, false);
        for _ in 0..64 {
            match sample_bool(&mut rng) {
                true => t = true,
                false => f = true,
            }
            if t && f {
                return;
            }
        }
        panic!("bool samples covered only one value");
    }

    #[test]
    fn u8_covers_both_halves_of_range() {
        let mut rng = Rng::new(1);
        let (mut low, mut high) = (false, false);
        for _ in 0..64 {
            let v = sample_u8(&mut rng);
            low |= v < 128;
            high |= v >= 128;
            if low && high {
                return;
            }
        }
        panic!("u8 samples covered only one half of the range");
    }

    #[test]
    fn i8_can_be_negative_and_nonnegative() {
        let mut rng = Rng::new(1);
        let (mut neg, mut nonneg) = (false, false);
        for _ in 0..64 {
            let v = sample_i8(&mut rng);
            neg |= v < 0;
            nonneg |= v >= 0;
            if neg && nonneg {
                return;
            }
        }
        panic!("i8 samples covered only one sign");
    }

    #[test]
    fn non_zero_primitives_are_deterministic() {
        let mut a = Rng::new(456);
        let mut b = Rng::new(456);
        assert_eq!(sample_non_zero_u8(&mut a), sample_non_zero_u8(&mut b));
        assert_eq!(sample_non_zero_u16(&mut a), sample_non_zero_u16(&mut b));
        assert_eq!(sample_non_zero_u32(&mut a), sample_non_zero_u32(&mut b));
        assert_eq!(sample_non_zero_u64(&mut a), sample_non_zero_u64(&mut b));
        assert_eq!(sample_non_zero_u128(&mut a), sample_non_zero_u128(&mut b));
        assert_eq!(sample_non_zero_usize(&mut a), sample_non_zero_usize(&mut b));
        assert_eq!(sample_non_zero_i8(&mut a), sample_non_zero_i8(&mut b));
        assert_eq!(sample_non_zero_i16(&mut a), sample_non_zero_i16(&mut b));
        assert_eq!(sample_non_zero_i32(&mut a), sample_non_zero_i32(&mut b));
        assert_eq!(sample_non_zero_i64(&mut a), sample_non_zero_i64(&mut b));
        assert_eq!(sample_non_zero_i128(&mut a), sample_non_zero_i128(&mut b));
        assert_eq!(sample_non_zero_isize(&mut a), sample_non_zero_isize(&mut b));
    }

    #[test]
    fn non_zero_u8_exercises_rejection_loop() {
        // Type invariant already guarantees non-zero; this just exercises
        // the rejection loop over many samples without panicking.
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let _ = sample_non_zero_u8(&mut rng);
        }
    }

    #[test]
    fn sample_choice_returns_only_from_slice() {
        let mut rng = Rng::new(1);
        let choices = [10, 20, 30];
        for _ in 0..256 {
            assert!(choices.contains(&sample_choice(&mut rng, &choices)));
        }
    }

    #[test]
    fn sample_choice_can_hit_every_element() {
        let mut rng = Rng::new(1);
        let choices = [10, 20, 30];
        let mut seen = [false; 3];
        for _ in 0..256 {
            let v = sample_choice(&mut rng, &choices);
            let idx = choices.iter().position(|&x| x == v).unwrap();
            seen[idx] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_choice did not cover all elements");
    }

    #[test]
    #[should_panic(expected = "empty slice")]
    fn sample_choice_panics_on_empty() {
        let mut rng = Rng::new(0);
        let empty: [u32; 0] = [];
        let _ = sample_choice(&mut rng, &empty);
    }

    #[test]
    fn sample_choice_works_with_clone_only_types() {
        // Verify T: Clone + Debug bound accepts non-Copy types with Debug.
        let mut rng = Rng::new(1);
        let choices = vec![String::from("a"), String::from("b"), String::from("c")];
        let picked = sample_choice(&mut rng, &choices);
        assert!(choices.contains(&picked));
    }

    #[test]
    fn char_generators_are_deterministic() {
        let mut a = Rng::new(789);
        let mut b = Rng::new(789);
        assert_eq!(sample_char(&mut a), sample_char(&mut b));
        assert_eq!(sample_ascii_char(&mut a), sample_ascii_char(&mut b));
        assert_eq!(
            sample_ascii_printable_char(&mut a),
            sample_ascii_printable_char(&mut b)
        );
    }

    #[test]
    fn sample_ascii_char_always_in_ascii_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let c = sample_ascii_char(&mut rng);
            assert!(c.is_ascii());
        }
    }

    #[test]
    fn sample_ascii_printable_char_always_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let c = sample_ascii_printable_char(&mut rng);
            let n = c as u32;
            assert!((0x20..=0x7E).contains(&n));
        }
    }

    #[test]
    fn sample_char_produces_varied_values() {
        // Valid Unicode scalar space is ~1.1M chars, so 256 samples should
        // be nearly all distinct (collision probability is negligible).
        let mut rng = Rng::new(1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            seen.insert(sample_char(&mut rng));
        }
        assert!(
            seen.len() > 200,
            "sample_char produced too few distinct values: {}",
            seen.len()
        );
    }

    #[test]
    fn float_generators_are_deterministic() {
        let mut a = Rng::new(999);
        let mut b = Rng::new(999);
        assert_eq!(sample_f32(&mut a, 0.0, 1.0), sample_f32(&mut b, 0.0, 1.0));
        assert_eq!(
            sample_f64(&mut a, -100.0, 100.0),
            sample_f64(&mut b, -100.0, 100.0)
        );
    }

    #[test]
    fn sample_f32_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = sample_f32(&mut rng, 10.0, 20.0);
            assert!((10.0..20.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn sample_f64_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = sample_f64(&mut rng, -1.0, 1.0);
            assert!((-1.0..1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn sample_f32_covers_both_halves_of_range() {
        let mut rng = Rng::new(1);
        let (mut low, mut high) = (false, false);
        for _ in 0..64 {
            let v = sample_f32(&mut rng, 0.0, 1.0);
            low |= v < 0.5;
            high |= v >= 0.5;
            if low && high {
                return;
            }
        }
        panic!("sample_f32 covered only one half of the range");
    }

    #[test]
    #[should_panic(expected = "must be less than")]
    fn sample_f32_panics_when_min_equals_max() {
        let mut rng = Rng::new(0);
        let _ = sample_f32(&mut rng, 5.0, 5.0);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn sample_f32_panics_on_nan() {
        let mut rng = Rng::new(0);
        let _ = sample_f32(&mut rng, f32::NAN, 1.0);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn sample_f32_panics_on_infinity() {
        let mut rng = Rng::new(0);
        let _ = sample_f32(&mut rng, 0.0, f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn sample_f64_panics_on_nan() {
        let mut rng = Rng::new(0);
        let _ = sample_f64(&mut rng, 0.0, f64::NAN);
    }

    // === sample_below ===

    #[test]
    fn sample_below_one_returns_zero_without_drawing() {
        // n == 1 has a single legal value, so no RNG bytes must be consumed.
        let mut rng = Rng::new(1);
        let mut fresh = Rng::new(1);
        assert_eq!(sample_below(&mut rng, 1), 0);
        assert_eq!(rng.next_u64(), fresh.next_u64());
    }

    #[test]
    fn sample_below_stays_in_range() {
        let mut rng = Rng::new(42);
        for _ in 0..10_000 {
            let v = sample_below(&mut rng, 7);
            assert!(v < 7);
        }
    }

    #[test]
    fn sample_below_hits_every_value() {
        let mut rng = Rng::new(42);
        let mut seen = [false; 5];
        for _ in 0..1024 {
            let v = sample_below(&mut rng, 5) as usize;
            seen[v] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_below did not cover all values");
    }

    #[test]
    fn sample_below_is_roughly_uniform_on_non_divisor() {
        // 3 does not divide 2^64, so the sampler must actively reject.
        // Verify that a large batch of draws is roughly balanced.
        let mut rng = Rng::new(7);
        let mut counts = [0usize; 3];
        let total = 30_000;
        for _ in 0..total {
            counts[sample_below(&mut rng, 3) as usize] += 1;
        }
        // Expected ~10_000 per bucket. Allow a very generous slack.
        let expected = total / 3;
        for c in counts {
            assert!(
                c.abs_diff(expected) < expected / 10,
                "bucket count off: {c} vs {expected}"
            );
        }
    }

    #[test]
    fn sample_below_accepts_full_u64_domain() {
        // u64::MAX is odd (not a multiple of 2), so this exercises the
        // rejection path with the widest possible bound.
        let mut rng = Rng::new(1);
        let _v = sample_below(&mut rng, u64::MAX);
    }

    // === sample_usize_in ===

    #[test]
    fn sample_usize_in_exclusive_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = sample_usize_in(&mut rng, 10..20);
            assert!((10..20).contains(&v));
        }
    }

    #[test]
    fn sample_usize_in_inclusive_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = sample_usize_in(&mut rng, 10..=20);
            assert!((10..=20).contains(&v));
        }
    }

    #[test]
    fn sample_usize_in_single_element_returns_that_element() {
        let mut rng = Rng::new(1);
        // 5..=5 is one element; the runner should return it without
        // consuming any RNG.
        let mut fresh = Rng::new(1);
        assert_eq!(sample_usize_in(&mut rng, 5..=5), 5);
        assert_eq!(rng.next_u64(), fresh.next_u64());
    }

    #[test]
    fn sample_usize_in_hits_both_endpoints() {
        let mut rng = Rng::new(1);
        let (mut lo, mut hi) = (false, false);
        for _ in 0..1024 {
            let v = sample_usize_in(&mut rng, 0..=3);
            lo |= v == 0;
            hi |= v == 3;
            if lo && hi {
                return;
            }
        }
        panic!("sample_usize_in did not cover both endpoints");
    }

    #[test]
    fn sample_usize_in_full_range_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..100 {
            let _v = sample_usize_in(&mut rng, ..);
            // Any usize is in range; just verify no panic.
        }
    }

    #[test]
    fn sample_usize_in_inclusive_up_to_max_stays_in_range() {
        // Exercises the max - lo + 1 arithmetic on the widest non-full
        // range so it must not overflow.
        let mut rng = Rng::new(1);
        for _ in 0..100 {
            let v = sample_usize_in(&mut rng, 1..=usize::MAX);
            assert!(v >= 1);
        }
    }

    #[test]
    fn sample_usize_in_unbounded_end_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..100 {
            let v = sample_usize_in(&mut rng, 100..);
            assert!(v >= 100);
        }
    }

    #[test]
    fn sample_usize_in_is_deterministic() {
        let mut a = Rng::new(999);
        let mut b = Rng::new(999);
        for _ in 0..64 {
            assert_eq!(
                sample_usize_in(&mut a, 0..137),
                sample_usize_in(&mut b, 0..137)
            );
        }
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_empty_exclusive() {
        let mut rng = Rng::new(0);
        let _ = sample_usize_in(&mut rng, 5..5);
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_reversed_inclusive() {
        let mut rng = Rng::new(0);
        #[allow(clippy::reversed_empty_ranges)]
        let _ = sample_usize_in(&mut rng, 5..=4);
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_zero_exclusive_end() {
        let mut rng = Rng::new(0);
        let _ = sample_usize_in(&mut rng, ..0);
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn sample_usize_in_panics_on_excluded_max_start() {
        let mut rng = Rng::new(0);
        // An excluded start of usize::MAX would need start + 1, which
        // overflows — semantically the range is empty.
        let _ = sample_usize_in(
            &mut rng,
            (
                std::ops::Bound::Excluded(usize::MAX),
                std::ops::Bound::<usize>::Unbounded,
            ),
        );
    }

    // === sample_ratio ===

    #[test]
    fn sample_ratio_zero_numerator_always_false_and_draws_nothing() {
        let mut rng = Rng::new(1);
        let mut fresh = Rng::new(1);
        for _ in 0..64 {
            assert!(!sample_ratio(&mut rng, 0, 10));
        }
        // No RNG bytes consumed.
        assert_eq!(rng.next_u64(), fresh.next_u64());
    }

    #[test]
    fn sample_ratio_full_numerator_always_true_and_draws_nothing() {
        let mut rng = Rng::new(1);
        let mut fresh = Rng::new(1);
        for _ in 0..64 {
            assert!(sample_ratio(&mut rng, 7, 7));
        }
        assert_eq!(rng.next_u64(), fresh.next_u64());
    }

    #[test]
    fn sample_ratio_produces_both_outcomes() {
        let mut rng = Rng::new(1);
        let (mut t, mut f) = (false, false);
        for _ in 0..256 {
            match sample_ratio(&mut rng, 1, 2) {
                true => t = true,
                false => f = true,
            }
            if t && f {
                return;
            }
        }
        panic!("sample_ratio did not produce both outcomes");
    }

    #[test]
    fn sample_ratio_is_deterministic() {
        let mut a = Rng::new(999);
        let mut b = Rng::new(999);
        for _ in 0..64 {
            assert_eq!(sample_ratio(&mut a, 3, 7), sample_ratio(&mut b, 3, 7));
        }
    }

    #[test]
    fn sample_ratio_biased_matches_expected_frequency() {
        // 1-in-10 draws should sit near 10% out of 10_000 samples.
        let mut rng = Rng::new(1);
        let mut trues: usize = 0;
        let total: usize = 10_000;
        for _ in 0..total {
            if sample_ratio(&mut rng, 1, 10) {
                trues += 1;
            }
        }
        let expected = total / 10;
        assert!(
            trues.abs_diff(expected) < expected / 2,
            "sample_ratio(1, 10) frequency off: {trues}/{total}"
        );
    }

    #[test]
    #[should_panic(expected = "denominator must be non-zero")]
    fn sample_ratio_panics_on_zero_denominator() {
        let mut rng = Rng::new(0);
        let _ = sample_ratio(&mut rng, 0, 0);
    }

    #[test]
    #[should_panic(expected = "must be <= denominator")]
    fn sample_ratio_panics_when_numerator_exceeds_denominator() {
        let mut rng = Rng::new(0);
        let _ = sample_ratio(&mut rng, 11, 10);
    }

    // === sample_weighted_index ===

    #[test]
    fn sample_weighted_index_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let idx = sample_weighted_index(&mut rng, &[1, 2, 3, 4]);
            assert!(idx < 4);
        }
    }

    #[test]
    fn sample_weighted_index_hits_every_nonzero_index() {
        let mut rng = Rng::new(1);
        let weights = [1, 1, 1];
        let mut seen = [false; 3];
        for _ in 0..1024 {
            seen[sample_weighted_index(&mut rng, &weights)] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("sample_weighted_index did not cover all non-zero indices");
    }

    #[test]
    fn sample_weighted_index_skips_zero_weight_slot() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let idx = sample_weighted_index(&mut rng, &[3, 0, 5]);
            assert_ne!(idx, 1, "index 1 has weight 0 and must never be picked");
        }
    }

    #[test]
    fn sample_weighted_index_single_nonzero_always_returns_it() {
        let mut rng = Rng::new(1);
        for _ in 0..100 {
            assert_eq!(sample_weighted_index(&mut rng, &[0, 0, 42, 0]), 2);
        }
    }

    #[test]
    fn sample_weighted_index_is_deterministic() {
        let mut a = Rng::new(123);
        let mut b = Rng::new(123);
        let weights = [4, 1, 2, 3];
        for _ in 0..64 {
            assert_eq!(
                sample_weighted_index(&mut a, &weights),
                sample_weighted_index(&mut b, &weights)
            );
        }
    }

    #[test]
    fn sample_weighted_index_frequencies_approximate_weights() {
        // Weights 1:2:3 → ~1/6, 2/6, 3/6 of samples.
        let mut rng = Rng::new(1);
        let weights = [1, 2, 3];
        let mut counts = [0usize; 3];
        let total = 12_000;
        for _ in 0..total {
            counts[sample_weighted_index(&mut rng, &weights)] += 1;
        }
        // Expected 2000 / 4000 / 6000. Allow ±30% slack.
        for (i, (&c, &w)) in counts.iter().zip(weights.iter()).enumerate() {
            let expected = total * w as usize / 6;
            assert!(
                c.abs_diff(expected) * 100 < expected * 30,
                "index {i}: got {c}, expected ~{expected}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "empty weights")]
    fn sample_weighted_index_panics_on_empty() {
        let mut rng = Rng::new(0);
        let _ = sample_weighted_index(&mut rng, &[]);
    }

    #[test]
    #[should_panic(expected = "all weights are zero")]
    fn sample_weighted_index_panics_when_all_weights_zero() {
        let mut rng = Rng::new(0);
        let _ = sample_weighted_index(&mut rng, &[0, 0, 0]);
    }
}
