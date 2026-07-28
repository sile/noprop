//! Value generators.
//!
//! Every generator has the shape `fn gen_X(rng: &mut Rng) -> X`. User
//! code composes generators with plain Rust — closures, `match`, `for`,
//! iterators — so property tests read like ordinary Rust code, and user
//! generators (`fn gen_person(rng: &mut Rng) -> Person`) have the same
//! shape as the built-in ones.
//!
//! # Composing generators
//!
//! To build a generator whose output depends on another's, just call
//! them sequentially inside a plain function or closure:
//!
//! ```
//! use noprop::Rng;
//!
//! fn gen_bounded_vec(rng: &mut Rng) -> Vec<u32> {
//!     // Pick a length first, then a Vec of that length.
//!     let len = noprop::gen_u32(rng) % 10;
//!     (0..len).map(|_| noprop::gen_u32(rng)).collect()
//! }
//!
//! let mut rng = Rng::new(0);
//! let _v: Vec<u32> = gen_bounded_vec(&mut rng);
//! ```
//!
//! For "one-of-N" branching between code paths, `match` on a small
//! random value:
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! let _x: u32 = match noprop::gen_u8(&mut rng) % 3 {
//!     0 => 0,
//!     1 => noprop::gen_u32(&mut rng),
//!     _ => u32::MAX,
//! };
//! ```
//!
//! To pick one value from a fixed list, use [`gen_choice`]:
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! let _n = noprop::gen_choice(&mut rng, &[1, 2, 3, 5, 8]);
//! let _digit = noprop::gen_choice(&mut rng, b"0123456789") as char;
//! ```
//!
//! For bounded retry (filter-style generation), combine a range iterator
//! with `.find()`:
//!
//! ```
//! let mut rng = noprop::Rng::new(0);
//! let even: Option<u32> = (0..100)
//!     .map(|_| noprop::gen_u32(&mut rng))
//!     .find(|x| x % 2 == 0);
//! # assert!(even.is_some());
//! ```

use std::num::NonZero;
use std::panic::Location;

use crate::Rng;

/// Read `N` bytes from `rng` without recording. Used by every primitive
/// so that composite generators (non-zero variants, `gen_char`, floats,
/// `gen_choice`) can consume randomness without producing intermediate
/// trace entries for the raw byte source.
fn raw_bytes<const N: usize>(rng: &mut Rng) -> [u8; N] {
    let mut buf = [0u8; N];
    rng.fill(&mut buf);
    buf
}

// === Selection helper ===

/// Pick one element from `choices` uniformly at random.
///
/// This is the noprop counterpart to picking from a fixed list. Use it
/// when the alternatives are *values*; for branching between code paths
/// (calling different generators, taking different actions), use `match`
/// on `gen_u8(rng) % N` instead — see the module docstring.
///
/// # Panics
///
/// Panics if `choices` is empty.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// // Explicit list of ints
/// let _n = noprop::gen_choice(&mut rng, &[1, 2, 3, 5, 8]);
/// // ASCII digit from a byte string literal
/// let _d = noprop::gen_choice(&mut rng, b"0123456789") as char;
/// // Non-ASCII via array literal
/// let _c = noprop::gen_choice(&mut rng, &['α', 'β', 'γ']);
/// ```
#[track_caller]
pub fn gen_choice<T: Clone + std::fmt::Debug + 'static>(rng: &mut Rng, choices: &[T]) -> T {
    assert!(!choices.is_empty(), "gen_choice: empty slice");
    let loc = Location::caller();
    let idx = usize::from_le_bytes(raw_bytes(rng)) % choices.len();
    let v = choices[idx].clone();
    rng.record_generated(&v, loc);
    v
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
/// let key: [u8; 32] = noprop::gen_bytes(&mut rng);
/// assert_eq!(key.len(), 32);
/// ```
#[track_caller]
pub fn gen_bytes<const N: usize>(rng: &mut Rng) -> [u8; N] {
    let loc = Location::caller();
    let bytes = raw_bytes::<N>(rng);
    rng.record_generated(&bytes, loc);
    bytes
}

/// Uniformly-distributed `Vec<u8>` of length `len`.
///
/// Use this when the byte-buffer length is known only at runtime
/// (`gen_bytes_vec(rng, gen_u32(rng) as usize % 1024)`). The whole
/// buffer is recorded as a single trace entry.
///
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// let bytes = noprop::gen_bytes_vec(&mut rng, 100);
/// assert_eq!(bytes.len(), 100);
/// ```
#[track_caller]
pub fn gen_bytes_vec(rng: &mut Rng, len: usize) -> Vec<u8> {
    let loc = Location::caller();
    let mut bytes = vec![0u8; len];
    rng.fill(&mut bytes);
    rng.record_generated(&bytes, loc);
    bytes
}

// === Boolean generator ===

/// Uniformly-distributed `bool`.
#[track_caller]
pub fn gen_bool(rng: &mut Rng) -> bool {
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
pub fn gen_u8(rng: &mut Rng) -> u8 {
    let loc = Location::caller();
    let v = raw_bytes::<1>(rng)[0];
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u16`.
#[track_caller]
pub fn gen_u16(rng: &mut Rng) -> u16 {
    let loc = Location::caller();
    let v = u16::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u32`.
#[track_caller]
pub fn gen_u32(rng: &mut Rng) -> u32 {
    let loc = Location::caller();
    let v = u32::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u64`.
#[track_caller]
pub fn gen_u64(rng: &mut Rng) -> u64 {
    let loc = Location::caller();
    let v = u64::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `u128`.
#[track_caller]
pub fn gen_u128(rng: &mut Rng) -> u128 {
    let loc = Location::caller();
    let v = u128::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `usize`.
#[track_caller]
pub fn gen_usize(rng: &mut Rng) -> usize {
    let loc = Location::caller();
    let v = usize::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i8`.
#[track_caller]
pub fn gen_i8(rng: &mut Rng) -> i8 {
    let loc = Location::caller();
    let v = raw_bytes::<1>(rng)[0] as i8;
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i16`.
#[track_caller]
pub fn gen_i16(rng: &mut Rng) -> i16 {
    let loc = Location::caller();
    let v = i16::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i32`.
#[track_caller]
pub fn gen_i32(rng: &mut Rng) -> i32 {
    let loc = Location::caller();
    let v = i32::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i64`.
#[track_caller]
pub fn gen_i64(rng: &mut Rng) -> i64 {
    let loc = Location::caller();
    let v = i64::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `i128`.
#[track_caller]
pub fn gen_i128(rng: &mut Rng) -> i128 {
    let loc = Location::caller();
    let v = i128::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed `isize`.
#[track_caller]
pub fn gen_isize(rng: &mut Rng) -> isize {
    let loc = Location::caller();
    let v = isize::from_le_bytes(raw_bytes(rng));
    rng.record_generated(&v, loc);
    v
}

// === Non-zero integer generators ===
//
// Each `gen_non_zero_*` uses rejection sampling: read the underlying
// integer and retry on zero. P(zero) is at most 1/256 per attempt for
// every type below, so the 64-attempt bound is effectively unreachable
// (worst-case P(all zero) < (1/256)^64 ~ 10^-154 for u8; even smaller
// elsewhere). Intermediate rejected attempts are not recorded — only
// the final NonZero value is.

/// Uniformly-distributed non-zero `u8`.
#[track_caller]
pub fn gen_non_zero_u8(rng: &mut Rng) -> NonZero<u8> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(raw_bytes::<1>(rng)[0]) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_u8: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u16`.
#[track_caller]
pub fn gen_non_zero_u16(rng: &mut Rng) -> NonZero<u16> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u16::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_u16: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u32`.
#[track_caller]
pub fn gen_non_zero_u32(rng: &mut Rng) -> NonZero<u32> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u32::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_u32: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u64`.
#[track_caller]
pub fn gen_non_zero_u64(rng: &mut Rng) -> NonZero<u64> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u64::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_u64: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u128`.
#[track_caller]
pub fn gen_non_zero_u128(rng: &mut Rng) -> NonZero<u128> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(u128::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_u128: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `usize`.
#[track_caller]
pub fn gen_non_zero_usize(rng: &mut Rng) -> NonZero<usize> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(usize::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_usize: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i8`.
#[track_caller]
pub fn gen_non_zero_i8(rng: &mut Rng) -> NonZero<i8> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(raw_bytes::<1>(rng)[0] as i8) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_i8: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i16`.
#[track_caller]
pub fn gen_non_zero_i16(rng: &mut Rng) -> NonZero<i16> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i16::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_i16: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i32`.
#[track_caller]
pub fn gen_non_zero_i32(rng: &mut Rng) -> NonZero<i32> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i32::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_i32: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i64`.
#[track_caller]
pub fn gen_non_zero_i64(rng: &mut Rng) -> NonZero<i64> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i64::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_i64: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i128`.
#[track_caller]
pub fn gen_non_zero_i128(rng: &mut Rng) -> NonZero<i128> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(i128::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_i128: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `isize`.
#[track_caller]
pub fn gen_non_zero_isize(rng: &mut Rng) -> NonZero<isize> {
    let loc = Location::caller();
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(isize::from_le_bytes(raw_bytes(rng))) {
            rng.record_generated(&nz, loc);
            return nz;
        }
    }
    panic!("gen_non_zero_isize: rejection sampling exhausted")
}

// === Character generators ===
//
// For character subsets beyond the ones below (alphanumeric, hexdigit,
// etc.), compose with `gen_choice` over a byte-string literal, for
// example:
//
//     let d = gen_choice(rng, b"0123456789") as char;
//     let a = gen_choice(rng, b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789") as char;

/// Uniformly-distributed `char` over the valid Unicode scalar values
/// (`0..=0x10FFFF`, excluding the surrogate range `0xD800..=0xDFFF`).
#[track_caller]
pub fn gen_char(rng: &mut Rng) -> char {
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
    panic!("gen_char: rejection sampling exhausted")
}

/// Uniformly-distributed ASCII `char` (`0x00..=0x7F`, including control
/// characters).
#[track_caller]
pub fn gen_ascii_char(rng: &mut Rng) -> char {
    let loc = Location::caller();
    let v = (raw_bytes::<1>(rng)[0] & 0x7F) as char;
    rng.record_generated(&v, loc);
    v
}

/// Uniformly-distributed printable ASCII `char` (`0x20..=0x7E`, space
/// through `~`, excluding control characters and DEL).
#[track_caller]
pub fn gen_ascii_printable_char(rng: &mut Rng) -> char {
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
/// [`gen_choice`]:
///
/// ```
/// let mut rng = noprop::Rng::new(0);
/// let _x = noprop::gen_choice(
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
/// let _x = f32::from_bits(noprop::gen_u32(&mut rng));
/// ```
///
/// # Panics
///
/// Panics if `min` or `max` is not finite, or if `min >= max`.
#[track_caller]
pub fn gen_f32(rng: &mut Rng, min: f32, max: f32) -> f32 {
    assert!(
        min.is_finite() && max.is_finite(),
        "gen_f32: min and max must be finite"
    );
    assert!(min < max, "gen_f32: min must be less than max");
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
/// Same conventions as [`gen_f32`]: NaN and infinities are excluded from
/// the output. Use [`gen_choice`] to include specific special values, or
/// `f64::from_bits(gen_u64(rng))` for an arbitrary bit pattern.
///
/// # Panics
///
/// Panics if `min` or `max` is not finite, or if `min >= max`.
#[track_caller]
pub fn gen_f64(rng: &mut Rng, min: f64, max: f64) -> f64 {
    assert!(
        min.is_finite() && max.is_finite(),
        "gen_f64: min and max must be finite"
    );
    assert!(min < max, "gen_f64: min must be less than max");
    let loc = Location::caller();
    // Same construction as gen_f32 but with 53-bit precision.
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
        assert_eq!(gen_bool(&mut a), gen_bool(&mut b));
        assert_eq!(gen_u8(&mut a), gen_u8(&mut b));
        assert_eq!(gen_u16(&mut a), gen_u16(&mut b));
        assert_eq!(gen_u32(&mut a), gen_u32(&mut b));
        assert_eq!(gen_u64(&mut a), gen_u64(&mut b));
        assert_eq!(gen_u128(&mut a), gen_u128(&mut b));
        assert_eq!(gen_usize(&mut a), gen_usize(&mut b));
        assert_eq!(gen_i8(&mut a), gen_i8(&mut b));
        assert_eq!(gen_i16(&mut a), gen_i16(&mut b));
        assert_eq!(gen_i32(&mut a), gen_i32(&mut b));
        assert_eq!(gen_i64(&mut a), gen_i64(&mut b));
        assert_eq!(gen_i128(&mut a), gen_i128(&mut b));
        assert_eq!(gen_isize(&mut a), gen_isize(&mut b));
    }

    #[test]
    fn bool_produces_both_values() {
        let mut rng = Rng::new(1);
        let (mut t, mut f) = (false, false);
        for _ in 0..64 {
            match gen_bool(&mut rng) {
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
            let v = gen_u8(&mut rng);
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
            let v = gen_i8(&mut rng);
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
        assert_eq!(gen_non_zero_u8(&mut a), gen_non_zero_u8(&mut b));
        assert_eq!(gen_non_zero_u16(&mut a), gen_non_zero_u16(&mut b));
        assert_eq!(gen_non_zero_u32(&mut a), gen_non_zero_u32(&mut b));
        assert_eq!(gen_non_zero_u64(&mut a), gen_non_zero_u64(&mut b));
        assert_eq!(gen_non_zero_u128(&mut a), gen_non_zero_u128(&mut b));
        assert_eq!(gen_non_zero_usize(&mut a), gen_non_zero_usize(&mut b));
        assert_eq!(gen_non_zero_i8(&mut a), gen_non_zero_i8(&mut b));
        assert_eq!(gen_non_zero_i16(&mut a), gen_non_zero_i16(&mut b));
        assert_eq!(gen_non_zero_i32(&mut a), gen_non_zero_i32(&mut b));
        assert_eq!(gen_non_zero_i64(&mut a), gen_non_zero_i64(&mut b));
        assert_eq!(gen_non_zero_i128(&mut a), gen_non_zero_i128(&mut b));
        assert_eq!(gen_non_zero_isize(&mut a), gen_non_zero_isize(&mut b));
    }

    #[test]
    fn non_zero_u8_exercises_rejection_loop() {
        // Type invariant already guarantees non-zero; this just exercises
        // the rejection loop over many samples without panicking.
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let _ = gen_non_zero_u8(&mut rng);
        }
    }

    #[test]
    fn gen_choice_returns_only_from_slice() {
        let mut rng = Rng::new(1);
        let choices = [10, 20, 30];
        for _ in 0..256 {
            assert!(choices.contains(&gen_choice(&mut rng, &choices)));
        }
    }

    #[test]
    fn gen_choice_can_hit_every_element() {
        let mut rng = Rng::new(1);
        let choices = [10, 20, 30];
        let mut seen = [false; 3];
        for _ in 0..256 {
            let v = gen_choice(&mut rng, &choices);
            let idx = choices.iter().position(|&x| x == v).unwrap();
            seen[idx] = true;
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        panic!("gen_choice did not cover all elements");
    }

    #[test]
    #[should_panic(expected = "empty slice")]
    fn gen_choice_panics_on_empty() {
        let mut rng = Rng::new(0);
        let empty: [u32; 0] = [];
        let _ = gen_choice(&mut rng, &empty);
    }

    #[test]
    fn gen_choice_works_with_clone_only_types() {
        // Verify T: Clone + Debug bound accepts non-Copy types with Debug.
        let mut rng = Rng::new(1);
        let choices = vec![String::from("a"), String::from("b"), String::from("c")];
        let picked = gen_choice(&mut rng, &choices);
        assert!(choices.contains(&picked));
    }

    #[test]
    fn char_generators_are_deterministic() {
        let mut a = Rng::new(789);
        let mut b = Rng::new(789);
        assert_eq!(gen_char(&mut a), gen_char(&mut b));
        assert_eq!(gen_ascii_char(&mut a), gen_ascii_char(&mut b));
        assert_eq!(
            gen_ascii_printable_char(&mut a),
            gen_ascii_printable_char(&mut b)
        );
    }

    #[test]
    fn gen_ascii_char_always_in_ascii_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let c = gen_ascii_char(&mut rng);
            assert!(c.is_ascii());
        }
    }

    #[test]
    fn gen_ascii_printable_char_always_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let c = gen_ascii_printable_char(&mut rng);
            let n = c as u32;
            assert!((0x20..=0x7E).contains(&n));
        }
    }

    #[test]
    fn gen_char_produces_varied_values() {
        // Valid Unicode scalar space is ~1.1M chars, so 256 samples should
        // be nearly all distinct (collision probability is negligible).
        let mut rng = Rng::new(1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            seen.insert(gen_char(&mut rng));
        }
        assert!(
            seen.len() > 200,
            "gen_char produced too few distinct values: {}",
            seen.len()
        );
    }

    #[test]
    fn float_generators_are_deterministic() {
        let mut a = Rng::new(999);
        let mut b = Rng::new(999);
        assert_eq!(gen_f32(&mut a, 0.0, 1.0), gen_f32(&mut b, 0.0, 1.0));
        assert_eq!(
            gen_f64(&mut a, -100.0, 100.0),
            gen_f64(&mut b, -100.0, 100.0)
        );
    }

    #[test]
    fn gen_f32_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = gen_f32(&mut rng, 10.0, 20.0);
            assert!((10.0..20.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn gen_f64_stays_in_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = gen_f64(&mut rng, -1.0, 1.0);
            assert!((-1.0..1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn gen_f32_covers_both_halves_of_range() {
        let mut rng = Rng::new(1);
        let (mut low, mut high) = (false, false);
        for _ in 0..64 {
            let v = gen_f32(&mut rng, 0.0, 1.0);
            low |= v < 0.5;
            high |= v >= 0.5;
            if low && high {
                return;
            }
        }
        panic!("gen_f32 covered only one half of the range");
    }

    #[test]
    #[should_panic(expected = "must be less than")]
    fn gen_f32_panics_when_min_equals_max() {
        let mut rng = Rng::new(0);
        let _ = gen_f32(&mut rng, 5.0, 5.0);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn gen_f32_panics_on_nan() {
        let mut rng = Rng::new(0);
        let _ = gen_f32(&mut rng, f32::NAN, 1.0);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn gen_f32_panics_on_infinity() {
        let mut rng = Rng::new(0);
        let _ = gen_f32(&mut rng, 0.0, f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn gen_f64_panics_on_nan() {
        let mut rng = Rng::new(0);
        let _ = gen_f64(&mut rng, 0.0, f64::NAN);
    }
}
