//! SplitMix64 (Steele, Lea & Flood, OOPSLA 2014): hand-rolled so a
//! recorded seed replays identically forever, independent of any
//! dependency's version-to-version stream changes (same precedent as
//! muaddib's `workload::rng::SplitMix64`).

#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// True with probability `p` in `[0.0, 1.0]`.
    pub fn chance(&mut self, p: f64) -> bool {
        // 53 bits of mantissa is plenty of resolution for fault probabilities.
        let frac = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        frac < p
    }

    /// Uniform in `0..n` (Lemire's multiply-shift). Panics if `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0)");
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn matches_published_reference_stream() {
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn chance_zero_and_one_are_exact() {
        let mut r = SplitMix64::new(7);
        for _ in 0..1000 {
            assert!(!r.chance(0.0));
        }
        for _ in 0..1000 {
            assert!(r.chance(1.0));
        }
    }

    #[test]
    fn below_stays_in_bounds() {
        let mut r = SplitMix64::new(9);
        for n in 1..40 {
            for _ in 0..20 {
                assert!(r.below(n) < n);
            }
        }
    }
}
