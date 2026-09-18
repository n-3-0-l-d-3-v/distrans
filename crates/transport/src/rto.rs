//! Adaptive retransmission timeout, in virtual ticks — Jacobson/Karels
//! estimation (the same smoothing TCP uses), with Karn's algorithm (never
//! sample RTT from a segment that was retransmitted, since an ACK for it
//! is ambiguous about which copy it acknowledges) and exponential backoff
//! on repeated timeouts.

/// Fixed-point scale for SRTT/RTTVAR, matching RFC 6298's integer-only
/// approach (avoids floats in a component whose whole point is
/// deterministic replay).
const SCALE: u64 = 8;

#[derive(Debug, Clone, Copy)]
pub struct RtoEstimator {
    srtt: Option<u64>, // scaled by SCALE
    rttvar: u64,       // scaled by SCALE / 2
    min_rto: u64,
    max_rto: u64,
    /// Multiplies the base RTO on consecutive timeouts without an
    /// intervening good sample; reset to 1 by `on_sample`.
    backoff: u32,
}

impl RtoEstimator {
    pub fn new(min_rto: u64, max_rto: u64) -> Self {
        Self {
            srtt: None,
            rttvar: 0,
            min_rto,
            max_rto,
            backoff: 1,
        }
    }

    /// The RTO to use *right now* for a freshly-sent (non-retransmitted)
    /// segment: the smoothed estimate, un-backed-off. Retransmissions use
    /// `current_backed_off` instead.
    fn base_rto(&self) -> u64 {
        let estimate = match self.srtt {
            Some(srtt) => (srtt / SCALE) + 4 * (self.rttvar / (SCALE / 2)).max(1),
            None => self.min_rto, // no sample yet: conservative default
        };
        estimate.clamp(self.min_rto, self.max_rto)
    }

    /// The RTO to arm a timer with right now, including backoff from any
    /// consecutive prior timeouts on this same segment.
    pub fn current(&self) -> u64 {
        (self.base_rto() * self.backoff as u64).min(self.max_rto)
    }

    /// Records a valid RTT sample (Karn's algorithm: caller must not call
    /// this for a retransmitted segment's ACK) and resets backoff.
    pub fn on_sample(&mut self, measured_rtt: u64) {
        self.backoff = 1;
        match self.srtt {
            None => {
                self.srtt = Some(measured_rtt * SCALE);
                self.rttvar = measured_rtt * SCALE / 2;
            }
            Some(srtt) => {
                let srtt_unscaled = srtt / SCALE;
                let diff = measured_rtt.abs_diff(srtt_unscaled);
                self.rttvar = self.rttvar - self.rttvar / 4 + diff * (SCALE / 2) / 4;
                self.srtt = Some(srtt - srtt / 8 + measured_rtt * SCALE / 8);
            }
        }
    }

    /// A timeout fired: double the backoff (capped by `max_rto`) for the
    /// next attempt at this segment.
    pub fn on_timeout(&mut self) {
        self.backoff = (self.backoff * 2).min(64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_samples_yet_rto_is_the_conservative_minimum() {
        let e = RtoEstimator::new(10, 1000);
        assert_eq!(e.current(), 10);
    }

    #[test]
    fn a_sample_shifts_the_estimate_toward_it() {
        let mut e = RtoEstimator::new(1, 1000);
        e.on_sample(50);
        // RTO should now be well above the bare RTT (SRTT + 4*RTTVAR).
        assert!(e.current() > 50);
        assert!(e.current() <= 1000);
    }

    #[test]
    fn repeated_consistent_samples_shrink_the_variance_and_thus_the_rto() {
        let mut e = RtoEstimator::new(1, 10_000);
        for _ in 0..50 {
            e.on_sample(100);
        }
        let converged = e.current();
        // With near-zero jitter the RTO should approach ~100, far below
        // an early estimate's wide variance-driven margin.
        assert!(converged < 150, "converged RTO {converged} too high");
        assert!(converged >= 100);
    }

    #[test]
    fn timeout_doubles_backoff_and_sample_resets_it() {
        let mut e = RtoEstimator::new(10, 100_000);
        e.on_sample(50);
        let base = e.current();
        e.on_timeout();
        assert_eq!(e.current(), base * 2);
        e.on_timeout();
        assert_eq!(e.current(), base * 4);
        // A fresh good sample resets the backoff multiplier to 1x, so the
        // RTO drops back to (at most) a fresh base estimate — not
        // necessarily identical to the first `base`, since a repeated
        // sample also legitimately shrinks the variance term (RTTVAR
        // converges toward the now-zero observed deviation), which this
        // checks doesn't happen: `current()` must drop, not merely stay
        // backed off.
        e.on_sample(50);
        assert!(
            e.current() <= base,
            "backoff did not reset: {} > {base}",
            e.current()
        );
    }

    #[test]
    fn backoff_and_rto_are_both_capped_by_max_rto() {
        let mut e = RtoEstimator::new(10, 200);
        e.on_sample(1000); // pushes base estimate above max_rto
        assert!(e.current() <= 200);
        for _ in 0..10 {
            e.on_timeout();
        }
        assert!(e.current() <= 200);
    }
}
