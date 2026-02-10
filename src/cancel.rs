// src/cancel.rs
use crate::task::{TaskId, WakeMsg, EXTERNAL_WAKE};
use crate::wake::WakeTx;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};


// src/cancel.rs (PATCH)
use crate::cancel_reason::{CancelReason, ABORT_PANICKED};
use std::sync::{
    atomic::{AtomicU8},
};

#[derive(Clone)]
pub struct CancelToken {
    id: TaskId,
    flag: Arc<AtomicBool>,
    wake: WakeTx,

    // NEW: only present for joinable tasks
    abort_reason: Option<Arc<AtomicU8>>,
}

impl CancelToken {
    pub(crate) fn new(id: TaskId, flag: Arc<AtomicBool>, wake: WakeTx, abort_reason: Option<Arc<AtomicU8>>) -> Self {
        Self { id, flag, wake, abort_reason }
    }

    #[inline]
    pub fn cancel(&self) {
        self.cancel_with_reason(CancelReason::UserRequest);
    }

    #[inline]
    pub fn cancel_with_reason(&self, reason: CancelReason) {
        self.flag.store(true, Ordering::SeqCst);

        // if joinable, set abort code only if not already set/panicked
        if let Some(ab) = self.abort_reason.as_ref() {
            let cur = ab.load(Ordering::Relaxed);
            if cur == 0 {
                ab.store(reason.as_u8(), Ordering::SeqCst);
            } else if cur == ABORT_PANICKED {
                // keep panicked
            }
        }

        self.wake.send(WakeMsg { from: EXTERNAL_WAKE, to: self.id });
    }

    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn task_id(&self) -> TaskId {
        self.id
    }
}