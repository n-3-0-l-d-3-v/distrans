//! What the hostile channel does to a datagram: the random `FaultProfile`
//! and the deterministic `Script` alternative, per
//! `docs/design/WIRE.md`'s "seeded hostility" model.

/// Independent per-datagram fault probabilities/parameters. Order of
/// application (see `Channel::send`): loss, then duplication, then
/// reordering delay, then corruption, then truncation — a dropped
/// datagram short-circuits the rest, everything else can compound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaultProfile {
    /// Probability a datagram is dropped outright.
    pub loss: f64,
    /// Probability a delivered datagram is additionally duplicated
    /// (the duplicate gets its own independent delay/corruption draw).
    pub duplication: f64,
    /// Extra virtual-tick delay is uniform in `0..=reorder_max_delay`,
    /// which can reorder datagrams relative to ones sent later with less
    /// delay. 0 disables reordering.
    pub reorder_max_delay: u64,
    /// Probability a delivered datagram has a random bit within its
    /// payload flipped (a no-op if the payload is empty).
    pub corruption: f64,
    /// Probability a delivered datagram is truncated to a random
    /// shorter length (including possibly 0).
    pub truncation: f64,
    /// Fixed base transit delay applied to every datagram that isn't
    /// dropped, on top of any reordering delay.
    pub base_delay: u64,
}

impl FaultProfile {
    /// No faults at all: a perfect, if not instant, channel.
    pub const CLEAN: FaultProfile = FaultProfile {
        loss: 0.0,
        duplication: 0.0,
        reorder_max_delay: 0,
        corruption: 0.0,
        truncation: 0.0,
        base_delay: 1,
    };

    pub fn validate(&self) -> Result<(), &'static str> {
        for (name, p) in [
            ("loss", self.loss),
            ("duplication", self.duplication),
            ("corruption", self.corruption),
            ("truncation", self.truncation),
        ] {
            if !(0.0..=1.0).contains(&p) {
                return Err(match name {
                    "loss" => "loss must be in [0.0, 1.0]",
                    "duplication" => "duplication must be in [0.0, 1.0]",
                    "corruption" => "corruption must be in [0.0, 1.0]",
                    _ => "truncation must be in [0.0, 1.0]",
                });
            }
        }
        Ok(())
    }
}

/// One deliberate, explicit fault applied to the `n`-th datagram sent on
/// a channel (0-indexed), independent of any random profile. For
/// deterministic regression tests of one specific corner case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptedFault {
    Drop,
    Duplicate,
    Delay(u64),
    Corrupt { byte_offset: usize },
    Truncate { to_len: usize },
}

/// An explicit list of `(datagram index, fault)` rules. Rules for the
/// same index apply in the order listed. Datagrams not named pass
/// through with the channel's `base_delay` and no other fault.
#[derive(Debug, Clone, Default)]
pub struct Script {
    rules: Vec<(u64, ScriptedFault)>,
}

impl Script {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn at(mut self, datagram_index: u64, fault: ScriptedFault) -> Self {
        self.rules.push((datagram_index, fault));
        self
    }

    pub fn for_index(&self, index: u64) -> impl Iterator<Item = ScriptedFault> + '_ {
        self.rules
            .iter()
            .filter(move |(i, _)| *i == index)
            .map(|(_, f)| *f)
    }
}

/// Counters for what actually happened, for observability
/// (`docs/DEFINITION_OF_DONE.md` item 7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaultStats {
    pub sent: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub duplicated: u64,
    pub reordered: u64,
    pub corrupted: u64,
    pub truncated: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_profile_validates() {
        assert!(FaultProfile::CLEAN.validate().is_ok());
    }

    #[test]
    fn out_of_range_probability_is_rejected() {
        let mut p = FaultProfile::CLEAN;
        p.loss = 1.5;
        assert!(p.validate().is_err());
    }

    #[test]
    fn script_returns_rules_for_the_right_index_only() {
        let s = Script::new()
            .at(3, ScriptedFault::Drop)
            .at(5, ScriptedFault::Delay(10));
        assert_eq!(
            s.for_index(3).collect::<Vec<_>>(),
            vec![ScriptedFault::Drop]
        );
        assert_eq!(s.for_index(4).count(), 0);
        assert_eq!(
            s.for_index(5).collect::<Vec<_>>(),
            vec![ScriptedFault::Delay(10)]
        );
    }

    #[test]
    fn multiple_rules_at_same_index_all_apply_in_order() {
        let s = Script::new()
            .at(0, ScriptedFault::Delay(2))
            .at(0, ScriptedFault::Corrupt { byte_offset: 0 });
        assert_eq!(s.for_index(0).count(), 2);
    }
}
