//! Value generation: the [`Generate`] trait and built-in generators.

use std::num::NonZero;

use crate::Rng;

/// A value generator.
///
/// noprop keeps this trait deliberately minimal — a single method plus a
/// blanket impl for closures. Composition happens in plain Rust
/// (closures, `match`, `for`, `loop`, iterators) rather than through a
/// combinator DSL, so property tests read like ordinary Rust code.
///
/// # Composing generators
///
/// Any `Fn(&mut Rng) -> T` is a `Generate<Output = T>`. To build a
/// generator whose output depends on another's, just write a closure:
///
/// ```
/// use noprop::Generate;
///
/// let mut rng = noprop::Rng::new(0);
/// // Pick a length first, then a Vec of that length.
/// let dependent = |rng: &mut noprop::Rng| -> Vec<u32> {
///     let len = noprop::u32().generate(rng) % 10;
///     (0..len).map(|_| noprop::u32().generate(rng)).collect()
/// };
/// let _v: Vec<u32> = dependent.generate(&mut rng);
/// ```
///
/// For branching, `match` on a small random value:
///
/// ```
/// use noprop::Generate;
///
/// let mut rng = noprop::Rng::new(0);
/// let _x: u32 = match noprop::u8().generate(&mut rng) % 3 {
///     0 => 0,
///     1 => noprop::u32().generate(&mut rng),
///     _ => u32::MAX,
/// };
/// ```
///
/// For bounded retry (filter-style generation), combine a range iterator
/// with `.find()`:
///
/// ```
/// use noprop::Generate;
///
/// let mut rng = noprop::Rng::new(0);
/// let even: Option<u32> = (0..100)
///     .map(|_| noprop::u32().generate(&mut rng))
///     .find(|x| x % 2 == 0);
/// # assert!(even.is_some());
/// ```
pub trait Generate {
    type Output;

    fn generate(&self, rng: &mut Rng) -> Self::Output;
}

/// Any `Fn(&mut Rng) -> T` is itself a generator.
///
/// This is what makes plain closures usable wherever a
/// `Generate<Output = T>` is expected, keeping the trait surface minimal.
impl<F, T> Generate for F
where
    F: Fn(&mut Rng) -> T,
{
    type Output = T;

    fn generate(&self, rng: &mut Rng) -> T {
        self(rng)
    }
}

// === Boolean generator ===

/// A generator that emits uniformly-distributed `bool` values.
///
/// Consumes one byte from the RNG per call to preserve the invariant
/// that every primitive draws a fixed-size byte slice (see [`Rng::fill`]).
pub fn bool() -> impl Generate<Output = bool> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 1];
        rng.fill(&mut buf);
        buf[0] & 1 != 0
    }
}

// === Integer generators ===
//
// All primitives draw randomness through `Rng::fill` (LE bytes ->
// `from_le_bytes`) so that every primitive consumes a fixed-size byte
// slice from the RNG. This keeps every generator compatible with a
// future bytes-based shrink implementation that swaps the RNG for a
// byte reader.

/// A generator that emits uniformly-distributed `u8` values.
pub fn u8() -> impl Generate<Output = u8> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 1];
        rng.fill(&mut buf);
        buf[0]
    }
}

/// A generator that emits uniformly-distributed `u16` values.
pub fn u16() -> impl Generate<Output = u16> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 2];
        rng.fill(&mut buf);
        u16::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `u32` values.
pub fn u32() -> impl Generate<Output = u32> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 4];
        rng.fill(&mut buf);
        u32::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `u64` values.
pub fn u64() -> impl Generate<Output = u64> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 8];
        rng.fill(&mut buf);
        u64::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `u128` values.
pub fn u128() -> impl Generate<Output = u128> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 16];
        rng.fill(&mut buf);
        u128::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `usize` values.
pub fn usize() -> impl Generate<Output = usize> {
    |rng: &mut Rng| {
        let mut buf = [0u8; size_of::<usize>()];
        rng.fill(&mut buf);
        usize::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `i8` values.
pub fn i8() -> impl Generate<Output = i8> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 1];
        rng.fill(&mut buf);
        buf[0] as i8
    }
}

/// A generator that emits uniformly-distributed `i16` values.
pub fn i16() -> impl Generate<Output = i16> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 2];
        rng.fill(&mut buf);
        i16::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `i32` values.
pub fn i32() -> impl Generate<Output = i32> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 4];
        rng.fill(&mut buf);
        i32::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `i64` values.
pub fn i64() -> impl Generate<Output = i64> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 8];
        rng.fill(&mut buf);
        i64::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `i128` values.
pub fn i128() -> impl Generate<Output = i128> {
    |rng: &mut Rng| {
        let mut buf = [0u8; 16];
        rng.fill(&mut buf);
        i128::from_le_bytes(buf)
    }
}

/// A generator that emits uniformly-distributed `isize` values.
pub fn isize() -> impl Generate<Output = isize> {
    |rng: &mut Rng| {
        let mut buf = [0u8; size_of::<isize>()];
        rng.fill(&mut buf);
        isize::from_le_bytes(buf)
    }
}

// === Non-zero integer generators ===
//
// Each `non_zero_*` uses rejection sampling: read the underlying integer
// and retry on zero. P(zero) is at most 1/256 per attempt for every type
// below, so the 64-attempt bound is effectively unreachable (worst-case
// P(all zero) < (1/256)^64 ~ 10^-154 for u8; even smaller elsewhere).

/// A generator that emits uniformly-distributed non-zero `u8` values.
pub fn non_zero_u8() -> impl Generate<Output = NonZero<u8>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(u8().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_u8: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `u16` values.
pub fn non_zero_u16() -> impl Generate<Output = NonZero<u16>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(u16().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_u16: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `u32` values.
pub fn non_zero_u32() -> impl Generate<Output = NonZero<u32>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(u32().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_u32: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `u64` values.
pub fn non_zero_u64() -> impl Generate<Output = NonZero<u64>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(u64().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_u64: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `u128` values.
pub fn non_zero_u128() -> impl Generate<Output = NonZero<u128>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(u128().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_u128: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `usize` values.
pub fn non_zero_usize() -> impl Generate<Output = NonZero<usize>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(usize().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_usize: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `i8` values.
pub fn non_zero_i8() -> impl Generate<Output = NonZero<i8>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(i8().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_i8: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `i16` values.
pub fn non_zero_i16() -> impl Generate<Output = NonZero<i16>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(i16().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_i16: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `i32` values.
pub fn non_zero_i32() -> impl Generate<Output = NonZero<i32>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(i32().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_i32: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `i64` values.
pub fn non_zero_i64() -> impl Generate<Output = NonZero<i64>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(i64().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_i64: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `i128` values.
pub fn non_zero_i128() -> impl Generate<Output = NonZero<i128>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(i128().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_i128: rejection sampling exhausted")
    }
}

/// A generator that emits uniformly-distributed non-zero `isize` values.
pub fn non_zero_isize() -> impl Generate<Output = NonZero<isize>> {
    |rng: &mut Rng| {
        for _ in 0..64 {
            if let Some(nz) = NonZero::new(isize().generate(rng)) {
                return nz;
            }
        }
        panic!("non_zero_isize: rejection sampling exhausted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closures_implement_generate() {
        // Blanket impl: any Fn(&mut Rng) -> T works as Generate<Output = T>.
        fn take_generator<G: Generate<Output = u32>>(g: G, rng: &mut Rng) -> u32 {
            g.generate(rng)
        }
        let mut rng = Rng::new(1);
        let v = take_generator(|_rng: &mut Rng| 42u32, &mut rng);
        assert_eq!(v, 42);
    }

    #[test]
    fn primitives_are_deterministic() {
        let mut a = Rng::new(123);
        let mut b = Rng::new(123);
        assert_eq!(bool().generate(&mut a), bool().generate(&mut b));
        assert_eq!(u8().generate(&mut a), u8().generate(&mut b));
        assert_eq!(u16().generate(&mut a), u16().generate(&mut b));
        assert_eq!(u32().generate(&mut a), u32().generate(&mut b));
        assert_eq!(u64().generate(&mut a), u64().generate(&mut b));
        assert_eq!(u128().generate(&mut a), u128().generate(&mut b));
        assert_eq!(usize().generate(&mut a), usize().generate(&mut b));
        assert_eq!(i8().generate(&mut a), i8().generate(&mut b));
        assert_eq!(i16().generate(&mut a), i16().generate(&mut b));
        assert_eq!(i32().generate(&mut a), i32().generate(&mut b));
        assert_eq!(i64().generate(&mut a), i64().generate(&mut b));
        assert_eq!(i128().generate(&mut a), i128().generate(&mut b));
        assert_eq!(isize().generate(&mut a), isize().generate(&mut b));
    }

    #[test]
    fn bool_produces_both_values() {
        let mut rng = Rng::new(1);
        let (mut t, mut f) = (false, false);
        for _ in 0..64 {
            match bool().generate(&mut rng) {
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
            let v = u8().generate(&mut rng);
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
            let v = i8().generate(&mut rng);
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
        assert_eq!(
            non_zero_u8().generate(&mut a),
            non_zero_u8().generate(&mut b)
        );
        assert_eq!(
            non_zero_u16().generate(&mut a),
            non_zero_u16().generate(&mut b)
        );
        assert_eq!(
            non_zero_u32().generate(&mut a),
            non_zero_u32().generate(&mut b)
        );
        assert_eq!(
            non_zero_u64().generate(&mut a),
            non_zero_u64().generate(&mut b)
        );
        assert_eq!(
            non_zero_u128().generate(&mut a),
            non_zero_u128().generate(&mut b)
        );
        assert_eq!(
            non_zero_usize().generate(&mut a),
            non_zero_usize().generate(&mut b)
        );
        assert_eq!(
            non_zero_i8().generate(&mut a),
            non_zero_i8().generate(&mut b)
        );
        assert_eq!(
            non_zero_i16().generate(&mut a),
            non_zero_i16().generate(&mut b)
        );
        assert_eq!(
            non_zero_i32().generate(&mut a),
            non_zero_i32().generate(&mut b)
        );
        assert_eq!(
            non_zero_i64().generate(&mut a),
            non_zero_i64().generate(&mut b)
        );
        assert_eq!(
            non_zero_i128().generate(&mut a),
            non_zero_i128().generate(&mut b)
        );
        assert_eq!(
            non_zero_isize().generate(&mut a),
            non_zero_isize().generate(&mut b)
        );
    }

    #[test]
    fn non_zero_u8_exercises_rejection_loop() {
        // Type invariant already guarantees non-zero; this just exercises
        // the rejection loop over many samples without panicking.
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let _ = non_zero_u8().generate(&mut rng);
        }
    }
}
