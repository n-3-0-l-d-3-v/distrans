//! Virtual time. Per `docs/design/WIRE.md`, no layer in this repo may
//! read physical time — `Tick` is the only clock anything here is
//! allowed to consult, and it advances only when the simulator says so.

/// A point in virtual time. Starts at 0; advances only via `Channel`'s
/// own event loop, never via a wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(pub u64);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    pub fn advance(self, by: u64) -> Tick {
        Tick(self.0.saturating_add(by))
    }
}

impl std::ops::Sub for Tick {
    type Output = u64;
    fn sub(self, rhs: Tick) -> u64 {
        self.0.saturating_sub(rhs.0)
    }
}

impl std::fmt::Display for Tick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "t{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_and_subtract() {
        let a = Tick(10);
        let b = a.advance(5);
        assert_eq!(b, Tick(15));
        assert_eq!(b - a, 5);
        assert_eq!(a - b, 0); // saturates, never wraps negative
    }

    #[test]
    fn ordering_is_by_value() {
        assert!(Tick(3) < Tick(4));
        assert!(Tick::ZERO < Tick(1));
    }
}
