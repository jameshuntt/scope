//! Scheduler policy: the knobs that decide who gets polled when.
//!
//! [`QoSClass`] and the QoS-biased orderings are the meaningful choices;
//! `Fifo` and `Lifo` are kept as deliberately simpler baselines to compare
//! against.

use std::time::Duration;

/// Quality-of-service band. Higher priority is scheduled first under the biased
/// orderings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QoSClass {
    /// Lowest priority: housekeeping that can wait for idle time.
    Background,
    /// Bulk work with no latency requirement.
    Batch,
    /// The default for tasks spawned without a class.
    Standard,
    /// Latency-sensitive work that goes ahead of batch work.
    Interactive,
    /// Highest priority, scheduled ahead of everything else.
    Realtime,
}

impl QoSClass {
    /// Numeric rank, higher first: `Realtime` is 5, `Background` is 1.
    pub const fn priority(self) -> u8 {
        match self {
            QoSClass::Realtime => 5,
            QoSClass::Interactive => 4,
            QoSClass::Standard => 3,
            QoSClass::Batch => 2,
            QoSClass::Background => 1,
        }
    }
}

/// How the runnable set is ordered each scheduler round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeOrdering {
    /// Insertion order, no rotation. Simple; can starve late arrivals under
    /// some wake patterns.
    Fifo,
    /// Most-recently-runnable first. Starves early tasks — a weak baseline.
    Lifo,
    /// Rotate the runnable set by an advancing cursor. Fair, QoS-blind.
    RoundRobin,
    /// QoS priority first, then runnable age.
    BiasedQoS,
    /// QoS priority, then least-recently-polled (anti-starvation), then age.
    BiasedQoSThenAge,
}

impl WakeOrdering {
    /// Every ordering, for iterating over the candidates.
    pub const ALL: [WakeOrdering; 5] = [
        WakeOrdering::Fifo,
        WakeOrdering::Lifo,
        WakeOrdering::RoundRobin,
        WakeOrdering::BiasedQoS,
        WakeOrdering::BiasedQoSThenAge,
    ];

    /// Stable lower-case name, used in traces and reports.
    pub const fn name(self) -> &'static str {
        match self {
            WakeOrdering::Fifo => "fifo",
            WakeOrdering::Lifo => "lifo",
            WakeOrdering::RoundRobin => "round_robin",
            WakeOrdering::BiasedQoS => "biased_qos",
            WakeOrdering::BiasedQoSThenAge => "biased_qos_then_age",
        }
    }
}

/// A full scheduler configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerConfig {
    /// How the runnable set is ordered each round.
    pub wake_ordering: WakeOrdering,
    /// Maximum tasks polled per round. `usize::MAX` == poll the whole runnable
    /// set (ordering then only affects sequence). A small budget makes ordering
    /// decide *who gets CPU under contention* — where fair and starving policies
    /// diverge.
    pub max_polls_per_round: usize,
    /// QoS assigned to tasks spawned without an explicit class.
    pub default_qos: QoSClass,
    /// Default tick budget for a soft cancel (0 == no tick escalation).
    pub default_cancel_ticks: u64,
    /// Default wall-clock budget for a soft cancel.
    pub default_cancel_time: Option<Duration>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            wake_ordering: WakeOrdering::BiasedQoSThenAge,
            max_polls_per_round: usize::MAX,
            default_qos: QoSClass::Standard,
            default_cancel_ticks: 0,
            default_cancel_time: None,
        }
    }
}

impl SchedulerConfig {
    /// The default configuration with a different ordering.
    pub fn with_ordering(ordering: WakeOrdering) -> Self {
        Self { wake_ordering: ordering, ..Self::default() }
    }

    /// Cap the tasks polled per round: the contention knob.
    pub fn with_poll_budget(mut self, budget: usize) -> Self {
        self.max_polls_per_round = budget;
        self
    }
}

/// Resolved cancel budget after applying an escalation against scope defaults.
#[derive(Copy, Clone, Debug)]
pub(crate) struct CancelBudgets {
    pub tick_budget: u64,
    pub time_budget: Option<Duration>,
    pub immediate: bool,
}

pub(crate) fn resolve_cancel_budgets(
    default_tick: u64,
    default_time: Option<Duration>,
    esc: crate::cancel::CancelEscalation,
) -> CancelBudgets {
    use crate::cancel::{TickEscalation, TimeEscalation};
    let tick_budget = match esc.tick {
        TickEscalation::Default => default_tick,
        TickEscalation::None => 0,
        TickEscalation::After(n) => n,
    };
    let time_budget = match esc.time {
        TimeEscalation::Default => default_time,
        TimeEscalation::None => None,
        TimeEscalation::After(d) => Some(d),
        TimeEscalation::Immediate => Some(Duration::ZERO),
    };
    let immediate = matches!(esc.time, TimeEscalation::Immediate)
        || time_budget == Some(Duration::ZERO);
    CancelBudgets { tick_budget, time_budget, immediate }
}
