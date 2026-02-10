// src/wake.rs
use crate::task::WakeMsg;
use std::time::Duration;

#[derive(Debug)]
pub enum WakeRecvTimeout {
    Msg(WakeMsg),
    Timeout,
    Disconnected,
}

#[cfg(any(target_arch = "wasm32", feature = "local_wake"))]
mod imp {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    #[derive(Clone)]
    pub struct WakeTx {
        q: Rc<RefCell<VecDeque<WakeMsg>>>,
    }

    #[derive(Clone)]
    pub struct WakeRx {
        q: Rc<RefCell<VecDeque<WakeMsg>>>,
    }

    pub fn wake_channel() -> (WakeTx, WakeRx) {
        let q = Rc::new(RefCell::new(VecDeque::new()));
        (WakeTx { q: q.clone() }, WakeRx { q })
    }

    impl WakeTx {
        #[inline]
        pub fn send(&self, msg: WakeMsg) {
            self.q.borrow_mut().push_back(msg);
        }
    }

    impl WakeRx {
        #[inline]
        pub fn supports_blocking(&self) -> bool {
            false
        }

        #[inline]
        pub fn try_recv(&self) -> Option<WakeMsg> {
            self.q.borrow_mut().pop_front()
        }

        #[inline]
        pub fn recv(&self) -> Option<WakeMsg> {
            // no blocking on WASM/local
            self.try_recv()
        }

        #[inline]
        pub fn recv_timeout(&self, _dur: Duration) -> WakeRecvTimeout {
            match self.try_recv() {
                Some(m) => WakeRecvTimeout::Msg(m),
                None => WakeRecvTimeout::Timeout,
            }
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "local_wake")))]
mod imp {
    use super::*;
    use std::sync::mpsc;

    #[derive(Clone)]
    pub struct WakeTx {
        tx: mpsc::Sender<WakeMsg>,
    }

    pub struct WakeRx {
        rx: mpsc::Receiver<WakeMsg>,
    }

    pub fn wake_channel() -> (WakeTx, WakeRx) {
        let (tx, rx) = mpsc::channel();
        (WakeTx { tx }, WakeRx { rx })
    }

    impl WakeTx {
        #[inline]
        pub fn send(&self, msg: WakeMsg) {
            let _ = self.tx.send(msg);
        }
    }

    impl WakeRx {
        #[inline]
        pub fn supports_blocking(&self) -> bool {
            true
        }

        #[inline]
        pub fn try_recv(&self) -> Option<WakeMsg> {
            self.rx.try_recv().ok()
        }

        #[inline]
        pub fn recv(&self) -> Option<WakeMsg> {
            self.rx.recv().ok()
        }

        #[inline]
        pub fn recv_timeout(&self, dur: Duration) -> WakeRecvTimeout {
            match self.rx.recv_timeout(dur) {
                Ok(m) => WakeRecvTimeout::Msg(m),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => WakeRecvTimeout::Timeout,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => WakeRecvTimeout::Disconnected,
            }
        }
    }
}

pub use imp::*;