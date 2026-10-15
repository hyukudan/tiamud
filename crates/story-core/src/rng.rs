//! Local seeded SplitMix64 generator.
//!
//! The generator is implemented here so the narrative ruleset depends on no
//! external RNG crate and its sequence can never change under a dependency
//! upgrade. Wrapping arithmetic is intrinsic to the generator; it never touches
//! a game quantity directly.

/// Seeded deterministic generator. The whole state is one `u64`, persisted in
/// [`crate::StoryState::rng`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SplitMix64 {
    state: u64,
}

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// Bounded rejection attempts; the probability of exhausting them for any
/// supported die is below 2^-300, and the fallback stays deterministic.
const MAX_REJECTIONS: usize = 64;

impl SplitMix64 {
    pub(crate) const fn from_state(state: u64) -> Self {
        Self { state }
    }

    /// Derives an initial generator state from a session seed so that seed 0
    /// and neighbouring seeds still produce unrelated sequences.
    pub(crate) fn initial_state(seed: u64) -> u64 {
        let mut generator = Self::from_state(seed ^ 0x5374_6f72_7943_6f72);
        generator.next_u64()
    }

    pub(crate) const fn state(self) -> u64 {
        self.state
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..upper` by rejection sampling. `upper == 0`
    /// returns 0 rather than panicking.
    pub(crate) fn below(&mut self, upper: u64) -> u64 {
        if upper <= 1 {
            return 0;
        }
        let threshold = upper.wrapping_neg() % upper;
        for _ in 0..MAX_REJECTIONS {
            let value = self.next_u64();
            if value >= threshold {
                return value % upper;
            }
        }
        self.next_u64() % upper
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_is_stable() {
        // Pinned values: changing the generator would silently change every
        // saved story, so this must fail loudly instead.
        let mut generator = SplitMix64::from_state(0);
        assert_eq!(generator.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(generator.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn below_stays_in_range() {
        let mut generator = SplitMix64::from_state(SplitMix64::initial_state(7));
        let mut seen = [false; 6];
        for _ in 0..1000 {
            let value = generator.below(6);
            assert!(value < 6);
            seen[value as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(generator.below(0), 0);
        assert_eq!(generator.below(1), 0);
    }
}
