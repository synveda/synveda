//! A token bucket expressed as the generic cell rate algorithm over integer
//! nanoseconds on an injected monotonic clock.

/// A sustained rate with a burst allowance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rate {
    interval_nanos: u64,
    tolerance_nanos: u64,
}

impl Rate {
    /// `per_minute` units sustained, `burst` admitted back to back from rest.
    /// Both are at least one; callers validate their own upper bounds.
    pub(crate) const fn new(per_minute: u32, burst: u32) -> Option<Self> {
        if per_minute == 0 || burst == 0 {
            return None;
        }
        let interval_nanos = 60_000_000_000 / per_minute as u64;
        Some(Self {
            interval_nanos,
            tolerance_nanos: interval_nanos * (burst as u64 - 1),
        })
    }
}

/// The theoretical arrival time of the next conforming unit. A cell whose
/// time is not after `now` carries no debt and is indistinguishable from a
/// fresh one, which is what lets the identity table forget it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cell(u64);

impl Cell {
    /// `Err` carries how long until one unit would conform.
    pub(crate) fn check(self, rate: Rate, now: u64) -> Result<(), u64> {
        let debt = self.0.max(now) - now;
        if debt <= rate.tolerance_nanos {
            Ok(())
        } else {
            Err(debt - rate.tolerance_nanos)
        }
    }

    pub(crate) fn charge(&mut self, rate: Rate, now: u64) {
        self.0 = self.0.max(now).saturating_add(rate.interval_nanos);
    }

    pub(crate) const fn is_rested(self, now: u64) -> bool {
        self.0 <= now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    fn take(cell: &mut Cell, rate: Rate, now: u64) -> Result<(), u64> {
        cell.check(rate, now)?;
        cell.charge(rate, now);
        Ok(())
    }

    #[test]
    fn a_rested_cell_admits_exactly_its_burst_at_one_instant() {
        let rate = Rate::new(60, 5).expect("rate");
        let mut cell = Cell::default();
        let now = 10 * SECOND;
        for _ in 0..5 {
            take(&mut cell, rate, now).expect("within burst");
        }
        let wait = take(&mut cell, rate, now).expect_err("burst spent");
        assert_eq!(wait, SECOND, "one unit returns after one interval");
    }

    #[test]
    fn the_sustained_rate_refills_one_unit_per_interval() {
        let rate = Rate::new(120, 1).expect("rate");
        let mut cell = Cell::default();
        take(&mut cell, rate, SECOND).expect("first");
        assert!(take(&mut cell, rate, SECOND + SECOND / 4).is_err());
        take(&mut cell, rate, SECOND + SECOND / 2).expect("refilled after half a second");
    }

    #[test]
    fn checking_never_charges_and_rest_is_reached_after_the_debt_is_paid() {
        let rate = Rate::new(60, 2).expect("rate");
        let mut cell = Cell::default();
        take(&mut cell, rate, SECOND).expect("first");
        take(&mut cell, rate, SECOND).expect("second");
        for _ in 0..10 {
            assert!(cell.check(rate, SECOND).is_err());
        }
        assert!(!cell.is_rested(2 * SECOND));
        assert!(cell.is_rested(3 * SECOND));
        take(&mut cell, rate, 3 * SECOND).expect("rested");
    }

    #[test]
    fn zero_rates_and_bursts_are_not_rates() {
        assert!(Rate::new(0, 1).is_none());
        assert!(Rate::new(1, 0).is_none());
    }

    #[test]
    fn a_far_future_debt_saturates_instead_of_wrapping() {
        let rate = Rate::new(1, 1).expect("rate");
        let mut cell = Cell(u64::MAX - 1);
        cell.charge(rate, 0);
        assert_eq!(cell, Cell(u64::MAX));
        assert!(cell.check(rate, 0).is_err());
    }
}
