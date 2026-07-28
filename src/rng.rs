//! Non-cryptographic seedable PRNG (xoshiro256** with SplitMix64 seed expansion).

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
/// # Examples
///
/// ```
/// let mut rng = noprop::Rng::new(0xDEAD_BEEF);
/// let a = rng.next_u64();
/// let b = rng.next_u64();
/// assert_ne!(a, b);
/// ```
#[derive(Debug, Clone)]
pub struct Rng {
    state: [u64; 4],
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
        }
    }

    /// Advance the PRNG by one step and return the next 64 random bits.
    ///
    /// This is the raw `xoshiro256**` step function.
    pub fn next_u64(&mut self) -> u64 {
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
    pub fn fill(&mut self, dst: &mut [u8]) {
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
