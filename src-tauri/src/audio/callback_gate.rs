use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

const CLOSED: usize = 1usize << (usize::BITS - 1);

/// Admission and active count share one atomic, so closing cannot miss an entrant.
#[derive(Default)]
pub(super) struct CallbackGate(AtomicUsize);

pub(super) struct CallbackAdmission<'a>(&'a CallbackGate);

impl CallbackGate {
    pub(super) fn try_enter(&self) -> Option<CallbackAdmission<'_>> {
        let mut state = self.0.load(Ordering::Acquire);
        loop {
            if state & CLOSED != 0 {
                return None;
            }
            match self.0.compare_exchange_weak(
                state,
                state + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(CallbackAdmission(self)),
                Err(current) => state = current,
            }
        }
    }

    /// Only the serialized control thread waits. Failure leaves admission closed.
    pub(super) fn close_and_wait(&self, timeout: Duration) -> bool {
        self.0.fetch_or(CLOSED, Ordering::AcqRel);
        let deadline = Instant::now() + timeout;
        loop {
            if self.0.load(Ordering::Acquire) == CLOSED {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn reopen(&self) {
        self.0.fetch_and(!CLOSED, Ordering::Release);
    }
}

impl Drop for CallbackAdmission<'_> {
    fn drop(&mut self) {
        self.0 .0.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[test]
    fn maintenance_waits_for_inflight_callback_and_excludes_new_callbacks() {
        let gate = Arc::new(CallbackGate::default());
        let active = gate.try_enter().unwrap();
        assert!(!gate.close_and_wait(Duration::ZERO));
        assert!(gate.try_enter().is_none());
        drop(active);
        assert!(gate.close_and_wait(Duration::ZERO));
        assert!(gate.try_enter().is_none());
        gate.reopen();
        assert!(gate.try_enter().is_some());
    }

    #[test]
    fn callback_exit_unblocks_control_without_plugin_lock_contention() {
        let gate = Arc::new(CallbackGate::default());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();
        let callback_gate = gate.clone();
        let callback = std::thread::spawn(move || {
            let _admission = callback_gate.try_enter().unwrap();
            entered_tx.send(()).unwrap();
            exit_rx.recv().unwrap();
        });
        entered_rx.recv().unwrap();
        assert!(!gate.close_and_wait(Duration::ZERO));
        exit_tx.send(()).unwrap();
        assert!(gate.close_and_wait(Duration::from_secs(1)));
        callback.join().unwrap();
        for _ in 0..1000 {
            assert!(gate.try_enter().is_none());
        }
    }
}
