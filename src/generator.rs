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

use crate::Rng;

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
pub fn gen_choice<T: Clone>(rng: &mut Rng, choices: &[T]) -> T {
    assert!(!choices.is_empty(), "gen_choice: empty slice");
    choices[gen_usize(rng) % choices.len()].clone()
}

// === Boolean generator ===

/// Uniformly-distributed `bool`.
///
/// Consumes one byte from the RNG per call to preserve the invariant
/// that every primitive draws a fixed-size byte slice (see [`Rng::fill`]).
pub fn gen_bool(rng: &mut Rng) -> bool {
    let mut buf = [0u8; 1];
    rng.fill(&mut buf);
    buf[0] & 1 != 0
}

// === Integer generators ===
//
// All primitives draw randomness through `Rng::fill` (LE bytes ->
// `from_le_bytes`) so that every primitive consumes a fixed-size byte
// slice from the RNG. This keeps every generator compatible with a
// future bytes-based shrink implementation that swaps the RNG for a
// byte reader.

/// Uniformly-distributed `u8`.
pub fn gen_u8(rng: &mut Rng) -> u8 {
    let mut buf = [0u8; 1];
    rng.fill(&mut buf);
    buf[0]
}

/// Uniformly-distributed `u16`.
pub fn gen_u16(rng: &mut Rng) -> u16 {
    let mut buf = [0u8; 2];
    rng.fill(&mut buf);
    u16::from_le_bytes(buf)
}

/// Uniformly-distributed `u32`.
pub fn gen_u32(rng: &mut Rng) -> u32 {
    let mut buf = [0u8; 4];
    rng.fill(&mut buf);
    u32::from_le_bytes(buf)
}

/// Uniformly-distributed `u64`.
pub fn gen_u64(rng: &mut Rng) -> u64 {
    let mut buf = [0u8; 8];
    rng.fill(&mut buf);
    u64::from_le_bytes(buf)
}

/// Uniformly-distributed `u128`.
pub fn gen_u128(rng: &mut Rng) -> u128 {
    let mut buf = [0u8; 16];
    rng.fill(&mut buf);
    u128::from_le_bytes(buf)
}

/// Uniformly-distributed `usize`.
pub fn gen_usize(rng: &mut Rng) -> usize {
    let mut buf = [0u8; size_of::<usize>()];
    rng.fill(&mut buf);
    usize::from_le_bytes(buf)
}

/// Uniformly-distributed `i8`.
pub fn gen_i8(rng: &mut Rng) -> i8 {
    let mut buf = [0u8; 1];
    rng.fill(&mut buf);
    buf[0] as i8
}

/// Uniformly-distributed `i16`.
pub fn gen_i16(rng: &mut Rng) -> i16 {
    let mut buf = [0u8; 2];
    rng.fill(&mut buf);
    i16::from_le_bytes(buf)
}

/// Uniformly-distributed `i32`.
pub fn gen_i32(rng: &mut Rng) -> i32 {
    let mut buf = [0u8; 4];
    rng.fill(&mut buf);
    i32::from_le_bytes(buf)
}

/// Uniformly-distributed `i64`.
pub fn gen_i64(rng: &mut Rng) -> i64 {
    let mut buf = [0u8; 8];
    rng.fill(&mut buf);
    i64::from_le_bytes(buf)
}

/// Uniformly-distributed `i128`.
pub fn gen_i128(rng: &mut Rng) -> i128 {
    let mut buf = [0u8; 16];
    rng.fill(&mut buf);
    i128::from_le_bytes(buf)
}

/// Uniformly-distributed `isize`.
pub fn gen_isize(rng: &mut Rng) -> isize {
    let mut buf = [0u8; size_of::<isize>()];
    rng.fill(&mut buf);
    isize::from_le_bytes(buf)
}

// === Non-zero integer generators ===
//
// Each `gen_non_zero_*` uses rejection sampling: read the underlying
// integer and retry on zero. P(zero) is at most 1/256 per attempt for
// every type below, so the 64-attempt bound is effectively unreachable
// (worst-case P(all zero) < (1/256)^64 ~ 10^-154 for u8; even smaller
// elsewhere).

/// Uniformly-distributed non-zero `u8`.
pub fn gen_non_zero_u8(rng: &mut Rng) -> NonZero<u8> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_u8(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_u8: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u16`.
pub fn gen_non_zero_u16(rng: &mut Rng) -> NonZero<u16> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_u16(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_u16: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u32`.
pub fn gen_non_zero_u32(rng: &mut Rng) -> NonZero<u32> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_u32(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_u32: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u64`.
pub fn gen_non_zero_u64(rng: &mut Rng) -> NonZero<u64> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_u64(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_u64: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `u128`.
pub fn gen_non_zero_u128(rng: &mut Rng) -> NonZero<u128> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_u128(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_u128: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `usize`.
pub fn gen_non_zero_usize(rng: &mut Rng) -> NonZero<usize> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_usize(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_usize: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i8`.
pub fn gen_non_zero_i8(rng: &mut Rng) -> NonZero<i8> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_i8(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_i8: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i16`.
pub fn gen_non_zero_i16(rng: &mut Rng) -> NonZero<i16> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_i16(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_i16: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i32`.
pub fn gen_non_zero_i32(rng: &mut Rng) -> NonZero<i32> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_i32(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_i32: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i64`.
pub fn gen_non_zero_i64(rng: &mut Rng) -> NonZero<i64> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_i64(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_i64: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `i128`.
pub fn gen_non_zero_i128(rng: &mut Rng) -> NonZero<i128> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_i128(rng)) {
            return nz;
        }
    }
    panic!("gen_non_zero_i128: rejection sampling exhausted")
}

/// Uniformly-distributed non-zero `isize`.
pub fn gen_non_zero_isize(rng: &mut Rng) -> NonZero<isize> {
    for _ in 0..64 {
        if let Some(nz) = NonZero::new(gen_isize(rng)) {
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
///
/// Uses rejection sampling on a 21-bit mask; expected rejection rate is
/// about 47%, so the 64-attempt bound is unreachable in practice
/// (P(all 64 fail) < 10^-20).
pub fn gen_char(rng: &mut Rng) -> char {
    for _ in 0..64 {
        let n = gen_u32(rng) & 0x1F_FFFF;
        if let Some(c) = char::from_u32(n) {
            return c;
        }
    }
    panic!("gen_char: rejection sampling exhausted")
}

/// Uniformly-distributed ASCII `char` (`0x00..=0x7F`, including control
/// characters).
pub fn gen_ascii_char(rng: &mut Rng) -> char {
    (gen_u8(rng) & 0x7F) as char
}

/// Uniformly-distributed printable ASCII `char` (`0x20..=0x7E`, space
/// through `~`, excluding control characters and DEL).
pub fn gen_ascii_printable_char(rng: &mut Rng) -> char {
    // 95 characters. Use gen_u32 for negligible modulo bias
    // (2^32 mod 95 = 6, so bias factor is at most 1 + 1/45210182).
    (0x20 + gen_u32(rng) % 95) as u8 as char
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
        // Verify T: Clone bound accepts non-Copy types.
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
}
