//! SplitMix64 — hand-rolled for the same reason as `channel::rng`'s own
//! (private) copy: a recorded chaos seed must replay identically forever,
//! independent of any dependency's version-to-version RNG changes. This
//! crate needs its own random *choices* (which client, which op, which
//! key) — separate from `channel`'s own internal fault-injection RNG,
//! which is already seeded independently per `Channel`.

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

    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0)");
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    pub fn percent(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(1);
        for _ in 0..50 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn below_stays_in_bounds() {
        let mut r = SplitMix64::new(3);
        for n in 1..30 {
            for _ in 0..10 {
                assert!(r.below(n) < n);
            }
        }
    }
}
