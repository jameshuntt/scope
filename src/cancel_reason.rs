// src/cancel_reason.rs
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    None = 0,
    UserRequest = 1,
    ShortCircuit = 2,
    TimeoutTicks = 3,
    TimeoutTime = 4,
    Unknown = 255,
}

impl CancelReason {
    #[inline]
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
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    pub fn is_meaningful(self) -> bool {
        self != CancelReason::None
    }
}

// Joinable abort codes:
// 0 = unset/unknown
// 1..=4 = CancelReason::*
// 250 = panicked
pub const ABORT_PANICKED: u8 = 250;

// src/cancel_reason.rs (ADD)
impl CancelReason {
    #[inline]
    pub fn priority(self) -> u8 {
        match self {
            CancelReason::None => 0,
            CancelReason::Unknown => 1,

            // timeouts are intentionally weak
            CancelReason::TimeoutTicks | CancelReason::TimeoutTime => 5,

            CancelReason::UserRequest => 10,
            CancelReason::ShortCircuit => 20,
        }
    }

    #[inline]
    pub fn is_timeout(self) -> bool {
        matches!(self, CancelReason::TimeoutTicks | CancelReason::TimeoutTime)
    }
}

/// Returns true if `new` is allowed to replace `cur` (priority + your special rule).
#[inline]
pub fn should_replace_reason(cur: CancelReason, new: CancelReason) -> bool {
    // Never replace meaningful reason with a timeout
    if new.is_timeout() && cur.is_meaningful() && cur != CancelReason::Unknown {
        return false;
    }
    // Otherwise: higher priority wins
    new.priority() > cur.priority()
}