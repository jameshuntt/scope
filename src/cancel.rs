//! Cancellation vocabulary: reasons, escalation policy, and the shared cell a
//! task reads to learn it has been asked to stop.
//!
//! Two-axis escalation (ported from the original scope): a cancel can be a soft
//! request that hard-stops after N scheduler ticks, or after a wall-clock
//! duration, or immediately. Reasons carry a priority so a meaningful reason is
//! never clobbered by a bland timeout.

use crate::sync::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

/// Why a task is being cancelled. `#[repr(u8)]` so it round-trips through the
/// atomic cell in the shared [`CancelCell`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// No cancellation requested.
    None = 0,
    /// Cancelled explicitly by a task or by the scope owner.
    UserRequest = 1,
    /// Cancelled because a combinator short-circuited: a sibling failed or a `select` was decided.
    ShortCircuit = 2,
    /// A soft cancel that hard-stopped when its tick budget ran out.
    TimeoutTicks = 3,
    /// A soft cancel that hard-stopped when its wall-clock budget ran out.
    TimeoutTime = 4,
    /// A reason code the cell did not recognise; never produced by this crate.
    Unknown = 255,
}

impl CancelReason {
    #[inline]
    /// The reason stored under a raw code; unknown codes become [`CancelReason::Unknown`].
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => CancelReason::None,
            1 => CancelReason::UserRequest,
            2 => CancelReason::ShortCircuit,
            3 => CancelReason::TimeoutTicks,
            4 => CancelReason::TimeoutTime,
            _ => CancelReason::Unknown,
        }
    }

    #[inline]
    /// The raw code stored in the cancel cell.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    /// Whether a cancellation was requested at all (anything but `None`).
    pub fn is_meaningful(self) -> bool {
        self != CancelReason::None
    }

    #[inline]
    /// Whether the reason is one of the two escalation timeouts.
    pub fn is_timeout(self) -> bool {
        matches!(self, CancelReason::TimeoutTicks | CancelReason::TimeoutTime)
    }

    #[inline]
    /// Precedence when two reasons compete for the cell; see [`should_replace_reason`].
    pub fn priority(self) -> u8 {
        match self {
            CancelReason::None => 0,
            CancelReason::Unknown => 1,
            CancelReason::TimeoutTicks | CancelReason::TimeoutTime => 5,
            CancelReason::UserRequest => 10,
            CancelReason::ShortCircuit => 20,
        }
    }
}

/// Abort code stored in a join channel when a task ends without a value.
/// `0` = unset, `1..=4` map to [`CancelReason`], `250` = panicked.
pub const ABORT_PANICKED: u8 = 250;

/// May `new` overwrite `cur`? Higher priority wins, but a timeout never
/// replaces an already-meaningful non-timeout reason.
#[inline]
pub fn should_replace_reason(cur: CancelReason, new: CancelReason) -> bool {
    if new.is_timeout() && cur.is_meaningful() && cur != CancelReason::Unknown {
        return false;
    }
    new.priority() > cur.priority()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// When a soft cancel hard-stops, counted in scheduler ticks.
pub enum TickEscalation {
    /// Use the scope default.
    Default,
    /// Never hard-stop on ticks.
    None,
    /// Hard-stop after N ticks (0 == None).
    After(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// When a soft cancel hard-stops, counted in wall-clock time.
pub enum TimeEscalation {
    /// Use the scope default.
    Default,
    /// Never hard-stop on time.
    None,
    /// Hard-stop after this much wall-clock time.
    After(Duration),
    /// Hard-stop now.
    Immediate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// How a soft cancel escalates: a tick budget and a time budget, whichever fires first.
pub struct CancelEscalation {
    /// The tick axis.
    pub tick: TickEscalation,
    /// The wall-clock axis.
    pub time: TimeEscalation,
}

impl CancelEscalation {
    /// Both axes at the scope defaults.
    pub const DEFAULT: Self = Self { tick: TickEscalation::Default, time: TimeEscalation::Default };

    /// A soft cancel that never hard-stops on its own.
    pub const fn none() -> Self {
        Self { tick: TickEscalation::None, time: TimeEscalation::None }
    }

    /// Hard-stop after `n` scheduler ticks, with no time budget.
    pub const fn after_ticks(n: u64) -> Self {
        Self { tick: TickEscalation::After(n), time: TimeEscalation::None }
    }

    /// Hard-stop after `d`, with no tick budget.
    pub const fn after(d: Duration) -> Self {
        Self { tick: TickEscalation::None, time: TimeEscalation::After(d) }
    }

    /// Hard-stop at once.
    pub const fn immediate() -> Self {
        Self { tick: TickEscalation::None, time: TimeEscalation::Immediate }
    }
}

/// Shared, thread-safe cancellation state for one task. The task holds an
/// `Arc<CancelCell>` (read via `Cx::is_cancelled`); the scope holds the other
/// `Arc` and flips it. This is the atomic pair loom explores.
#[derive(Debug)]
pub struct CancelCell {
    flag: AtomicBool,
    reason: AtomicU8,
}

impl CancelCell {
    /// A cell with no cancellation requested.
    pub fn new() -> Self {
        Self { flag: AtomicBool::new(false), reason: AtomicU8::new(CancelReason::None.as_u8()) }
    }

    #[inline]
    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    #[inline]
    /// The reason currently stored.
    pub fn reason(&self) -> CancelReason {
        CancelReason::from_u8(self.reason.load(Ordering::Acquire))
    }

    /// Request cancellation with a reason, honoring priority. Returns the
    /// reason that ended up stored.
    pub fn request(&self, new: CancelReason) -> CancelReason {
        // Compare-and-swap the reason under the priority rule.
        let mut cur = self.reason.load(Ordering::Acquire);
        loop {
            let cur_reason = CancelReason::from_u8(cur);
            if !should_replace_reason(cur_reason, new) {
                break;
            }
            match self.reason.compare_exchange_weak(
                cur,
                new.as_u8(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(v) => cur = v,
            }
        }
        self.flag.store(true, Ordering::Release);
        self.reason()
    }
}

impl Default for CancelCell {
    fn default() -> Self {
        Self::new()
    }
}
