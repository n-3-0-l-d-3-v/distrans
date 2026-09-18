//! Named hostile-channel profiles the closing workload is driven
//! through, per `docs/design/WIRE.md` ticket 006.

use channel::FaultProfile;

pub fn clean() -> FaultProfile {
    FaultProfile::CLEAN
}

pub fn lossy() -> FaultProfile {
    FaultProfile {
        loss: 0.10,
        duplication: 0.02,
        reorder_max_delay: 1,
        corruption: 0.01,
        truncation: 0.01,
        base_delay: 1,
    }
}

pub fn high_latency_reordering() -> FaultProfile {
    FaultProfile {
        loss: 0.02,
        duplication: 0.05,
        reorder_max_delay: 25,
        corruption: 0.0,
        truncation: 0.0,
        base_delay: 5,
    }
}

pub fn bursty_corruption() -> FaultProfile {
    FaultProfile {
        loss: 0.02,
        duplication: 0.0,
        reorder_max_delay: 0,
        corruption: 0.25,
        truncation: 0.05,
        base_delay: 1,
    }
}

pub fn near_partition() -> FaultProfile {
    FaultProfile {
        loss: 0.6,
        duplication: 0.0,
        reorder_max_delay: 2,
        corruption: 0.05,
        truncation: 0.0,
        base_delay: 1,
    }
}

/// Every named profile, paired with its name — for driving a workload
/// through all of them in turn.
pub fn all() -> Vec<(&'static str, FaultProfile)> {
    vec![
        ("clean", clean()),
        ("lossy", lossy()),
        ("high_latency_reordering", high_latency_reordering()),
        ("bursty_corruption", bursty_corruption()),
        ("near_partition", near_partition()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_profile_validates() {
        for (name, p) in all() {
            assert!(p.validate().is_ok(), "{name}: {p:?}");
        }
    }
}
