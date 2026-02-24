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


#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_wake_channel_fifo() {
        let (tx, rx) = wake_channel();
        
        let msg1 = WakeMsg { from: 1, to: 10 };
        let msg2 = WakeMsg { from: 2, to: 20 };

        tx.send(msg1);
        tx.send(msg2);

        // Verify Order
        let r1 = rx.try_recv().expect("Should have msg1");
        let r2 = rx.try_recv().expect("Should have msg2");

        assert_eq!(r1.from, 1);
        assert_eq!(r2.from, 2);
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn test_wake_recv_timeout_logic() {
        let (tx, rx) = wake_channel();
        let timeout = Duration::from_millis(10);

        // 1. Test actual timeout
        match rx.recv_timeout(timeout) {
            WakeRecvTimeout::Timeout => {} // Expected
            _ => panic!("Expected timeout on empty channel"),
        }

        // 2. Test success within timeout
        tx.send(WakeMsg { from: 5, to: 55 });
        match rx.recv_timeout(timeout) {
            WakeRecvTimeout::Msg(m) => assert_eq!(m.from, 5),
            _ => panic!("Expected message"),
        }
    }

    #[test]
    fn test_blocking_capability() {
        let (_, rx) = wake_channel();
        
        #[cfg(any(target_arch = "wasm32", feature = "local_wake"))]
        {
            assert!(!rx.supports_blocking());
            // On WASM, recv() should return immediately even if empty
            assert!(rx.recv().is_none()); 
        }

        #[cfg(all(not(target_arch = "wasm32"), not(feature = "local_wake")))]
        {
            assert!(rx.supports_blocking());
            // We don't call rx.recv() here because it would hang the test thread!
        }
    }

    #[test]
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "local_wake")))]
    fn test_disconnection_threaded() {
        let (tx, rx) = wake_channel();
        drop(tx); // Close the channel

        match rx.recv_timeout(Duration::from_millis(1)) {
            WakeRecvTimeout::Disconnected => {} // Correct
            _ => panic!("Expected Disconnected state"),
        }
    }
}
