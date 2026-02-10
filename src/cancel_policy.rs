// src/cancel_policy.rs
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickEscalation {
    /// Use Scope default
    Default,
    /// Disable tick-based hard-stop
    None,
    /// Hard-stop after N ticks (0 treated as None)
    After(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeEscalation {
    /// Use Scope default
    Default,
    /// Disable time-based hard-stop
    None,
    /// Hard-stop after duration (0 treated as Immediate)
    After(Duration),
    /// Hard-stop immediately (Scope methods hard-stop now; Cx schedules “now”)
    Immediate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelEscalation {
    pub tick: TickEscalation,
    pub time: TimeEscalation,
}

impl CancelEscalation {
    pub const DEFAULT: Self = Self {
        tick: TickEscalation::Default,
        time: TimeEscalation::Default,
    };

    pub const fn no_escalation() -> Self {
        Self { tick: TickEscalation::None, time: TimeEscalation::None }
    }

    pub const fn ticks(n: u64) -> Self {
        Self { tick: TickEscalation::After(n), time: TimeEscalation::None }
    }

    pub const fn time(d: Duration) -> Self {
        Self { tick: TickEscalation::None, time: TimeEscalation::After(d) }
    }
}