//! The logical clock: scheduler rounds, counted.
//!
//! A [`Tick`] is the number of the scheduler round it names. The clock has one
//! advancement rule: **it advances by one at the start of every round**, and
//! nothing else moves it. A round starts once per [`crate::Scope::tick`] call
//! and once per loop of [`crate::Scope::run`] and `run_until`, idle rounds
//! included. The first round is tick 1; before any round the clock reads tick 0.
//!
//! This is the count the scope already kept for tick-based cancel escalation
//! ([`crate::CancelEscalation::after_ticks`]), now readable. Nothing reads
//! `Instant`, `SystemTime` or sleeps to move it. A run that uses only logical
//! timers is a function of three things: the sequence of wakes, the policy,
//! and this advancement rule. Run it again with the same three and it repeats
//! tick for tick. Wall-clock timers are outside that replay.
//!
//! The clock never wraps. It stops at `u64::MAX`: the round after the one at
//! `u64::MAX` is tick `u64::MAX` again, and so is every round after it. A
//! deadline past `u64::MAX` (`now + delta` overflowing) is `u64::MAX`, so the
//! timer fires in the first round at `u64::MAX`, or in the next round when it
//! was asked for there. Saturating, like [`Tick`] `+` [`TickDelta`] and the
//! tick-based cancel deadlines, keeps every timer firing and every outcome
//! deterministic; a clock that wrapped would read 0 and leave every pending
//! deadline unreachable.
//!
//! A task asks for a logical timer with [`crate::Cx::sleep_ticks`] or
//! [`crate::Cx::sleep_until_tick`] (or spawns a [`crate::TickSleepTask`]). Its
//! deadline is fired at the start of the round with that tick, before the wake
//! bus is drained, as a [`crate::WakeReason::Timer`] wake on the bus: it is
//! ordered with the round's other wakes by the policy, exactly like any wake.
//!
//! Mixing in wall-clock timers ([`crate::SleepTask`], `Cx::sleep_for`) keeps
//! this rule but makes the number of rounds spent waiting on the wall clock,
//! and so the tick at which later work happens, depend on elapsed time.

use std::fmt;
use std::ops::Add;

/// A point on the logical clock: the number of a scheduler round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(pub u64);

/// A distance on the logical clock, in scheduler rounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TickDelta(pub u64);

impl Tick {
    /// Before the first round.
    pub const ZERO: Tick = Tick(0);

    /// The rounds from `earlier` to `self`, zero when `earlier` is later.
    pub const fn saturating_since(self, earlier: Tick) -> TickDelta {
        TickDelta(self.0.saturating_sub(earlier.0))
    }
}

impl Add<TickDelta> for Tick {
    type Output = Tick;

    /// Saturates at `u64::MAX`.
    fn add(self, delta: TickDelta) -> Tick {
        Tick(self.0.saturating_add(delta.0))
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

impl fmt::Display for TickDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ticks", self.0)
    }
}
