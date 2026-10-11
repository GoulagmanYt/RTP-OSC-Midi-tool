//! FIFO admission reserves slots for releases without moving them ahead of
//! already admitted Note-Ons. Permits are returned on dequeue or cancellation.
use crossbeam_channel::{RecvTimeoutError, TryRecvError, TrySendError};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

/// Decide without discarding a newly admitted event while its reset is pending.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GenerationAction {
    Discard,
    Defer,
    Ready,
}
pub(crate) fn generation_action(
    event: (u64, u64),
    handled: (u64, u64),
    current: (u64, u64),
) -> GenerationAction {
    if event.0 < current.0 || event.1 < current.1 {
        GenerationAction::Discard
    } else if handled != current || event != handled {
        GenerationAction::Defer
    } else {
        GenerationAction::Ready
    }
}

pub trait Priority {
    fn critical(&self) -> bool;
}
impl Priority for crate::midi::MidiFrame {
    fn critical(&self) -> bool {
        crate::midi::is_critical_release_message(&self.data)
    }
}
impl Priority for crate::types::MidiNoteEvent {
    fn critical(&self) -> bool {
        !self.pressed
    }
}
impl Priority for crate::vst_worker::MidiWireFrame {
    fn critical(&self) -> bool {
        crate::midi::is_critical_release_message(self.bytes())
    }
}

#[derive(Default)]
pub(crate) struct Recovery {
    generation: AtomicU64,
    name: String,
    capacity: usize,
    max_depth: AtomicUsize,
    max_wait_us: AtomicU64,
    admitted: AtomicU64,
    consumed: AtomicU64,
    noncritical_drops: AtomicU64,
    critical_drops: AtomicU64,
    noncritical_rejections: AtomicU64,
    critical_rejections: AtomicU64,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueueDiagnostics {
    pub name: String,
    pub capacity: usize,
    pub depth: u64,
    pub max_depth: usize,
    pub noncritical_drops: u64,
    pub critical_drops: u64,
    #[serde(default)]
    pub noncritical_rejections: u64,
    #[serde(default)]
    pub critical_rejections: u64,
    pub resets: u64,
    pub max_wait_us: u64,
}
static QUEUES: std::sync::LazyLock<parking_lot::Mutex<Vec<std::sync::Weak<Recovery>>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));
pub fn diagnostics() -> Vec<QueueDiagnostics> {
    let mut queues = QUEUES.lock();
    queues.retain(|queue| queue.strong_count() > 0);
    queues
        .iter()
        .filter_map(|q| q.upgrade())
        .map(|q| q.snapshot())
        .collect()
}
impl Recovery {
    pub(crate) fn snapshot(&self) -> QueueDiagnostics {
        QueueDiagnostics {
            name: self.name.clone(),
            capacity: self.capacity,
            depth: self
                .admitted
                .load(Ordering::Relaxed)
                .saturating_sub(self.consumed.load(Ordering::Relaxed)),
            max_depth: self.max_depth.load(Ordering::Relaxed),
            noncritical_drops: self.noncritical_drops.load(Ordering::Relaxed),
            critical_drops: self.critical_drops.load(Ordering::Relaxed),
            noncritical_rejections: self.noncritical_rejections.load(Ordering::Relaxed),
            critical_rejections: self.critical_rejections.load(Ordering::Relaxed),
            resets: self.generation(),
            max_wait_us: self.max_wait_us.load(Ordering::Relaxed),
        }
    }
    #[track_caller]
    fn new(capacity: usize) -> Arc<Self> {
        let location = std::panic::Location::caller();
        let recovery = Arc::new(Self {
            name: format!("{}:{}", location.file(), location.line()),
            capacity,
            ..Self::default()
        });
        QUEUES.lock().push(Arc::downgrade(&recovery));
        recovery
    }
    fn rejected(&self, critical: bool) {
        if critical {
            &self.critical_rejections
        } else {
            &self.noncritical_rejections
        }
        .fetch_add(1, Ordering::Relaxed);
    }
    fn dropped(&self, critical: bool) {
        if critical {
            &self.critical_drops
        } else {
            &self.noncritical_drops
        }
        .fetch_add(1, Ordering::Relaxed);
    }
    fn admitted(&self, depth: usize) {
        self.admitted.fetch_add(1, Ordering::Relaxed);
        self.max_depth.fetch_max(depth, Ordering::Relaxed);
    }
    fn consumed(&self, queued_qpc: u64) {
        self.consumed.fetch_add(1, Ordering::Relaxed);
        self.max_wait_us.fetch_max(
            crate::vst_worker::protocol::qpc_elapsed_us(
                queued_qpc,
                crate::vst_worker::protocol::monotonic_qpc(),
            ),
            Ordering::Relaxed,
        );
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub(crate) fn request_reset(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

struct Admission {
    used: AtomicUsize,
    limit: usize,
}
struct Permit(Arc<Admission>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.used.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Entry<T> {
    queued_qpc: u64,
    value: T,
    _permit: Option<Permit>,
}
struct Preparation<T> {
    apply: Box<dyn Fn(&mut T) -> bool + Send + Sync>,
    valid: std::sync::atomic::AtomicBool,
}
pub struct Sender<T> {
    preparation: Arc<arc_swap::ArcSwapOption<Preparation<T>>>,
    recovery: Arc<Recovery>,
    tx: crossbeam_channel::Sender<Entry<T>>,
    admission: Arc<Admission>,
}
pub struct Receiver<T>(crossbeam_channel::Receiver<Entry<T>>, Arc<Recovery>);
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            preparation: self.preparation.clone(),
            recovery: self.recovery.clone(),
            tx: self.tx.clone(),
            admission: self.admission.clone(),
        }
    }
}
#[track_caller]
pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = crossbeam_channel::bounded(capacity);
    let recovery = Recovery::new(capacity);
    (
        Sender {
            preparation: Arc::new(arc_swap::ArcSwapOption::empty()),
            recovery: recovery.clone(),
            tx,
            admission: Arc::new(Admission {
                used: AtomicUsize::new(0),
                limit: capacity - crate::realtime::reserve(capacity),
            }),
        },
        Receiver(rx, recovery),
    )
}
impl<T> Sender<T> {
    pub(crate) fn set_preparation(&self, prepare: impl Fn(&mut T) -> bool + Send + Sync + 'static) {
        self.preparation.store(Some(Arc::new(Preparation {
            apply: Box::new(prepare),
            valid: std::sync::atomic::AtomicBool::new(true),
        })));
    }
    /// Invalidate every outstanding copy before advancing the routing epoch.
    pub(crate) fn invalidate_preparation(&self) {
        if let Some(prepare) = self.preparation.load().as_ref() {
            prepare.valid.store(false, Ordering::Release);
        }
    }
    pub(crate) fn prepare(&self, value: &mut T) -> bool {
        self.preparation.load().as_ref().is_none_or(|prepare| {
            prepare.valid.load(Ordering::Acquire)
                && (prepare.apply)(value)
                && prepare.valid.load(Ordering::Acquire)
        })
    }
}
impl<T: Priority> Sender<T> {
    pub(crate) fn recovery(&self) -> &Recovery {
        &self.recovery
    }
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.try_send_inner(value, true)
    }
    pub(crate) fn try_send_retryable(&self, value: T) -> Result<(), TrySendError<T>> {
        self.try_send_inner(value, false)
    }
    pub(crate) fn record_abandoned(&self, value: &T) {
        self.recovery.dropped(value.critical());
    }
    fn try_send_inner(&self, value: T, abandoned: bool) -> Result<(), TrySendError<T>> {
        let critical = value.critical();
        let permit = if critical {
            None
        } else {
            if self
                .admission
                .used
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    (used < self.admission.limit).then_some(used + 1)
                })
                .is_err()
            {
                self.recovery.rejected(false);
                if abandoned {
                    self.recovery.dropped(false);
                }
                return Err(TrySendError::Full(value));
            }
            Some(Permit(self.admission.clone()))
        };
        self.tx
            .try_send(Entry {
                queued_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                value,
                _permit: permit,
            })
            .map(|()| self.recovery.admitted(self.tx.len()))
            .map_err(|e| {
                self.recovery.rejected(critical);
                if abandoned {
                    self.recovery.dropped(critical);
                }
                match e {
                    TrySendError::Full(e) => TrySendError::Full(e.value),
                    TrySendError::Disconnected(e) => TrySendError::Disconnected(e.value),
                }
            })
    }
    pub fn send(&self, mut value: T) -> Result<(), crossbeam_channel::SendError<T>> {
        loop {
            match self.try_send_retryable(value) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(v)) => {
                    self.record_abandoned(&v);
                    return Err(crossbeam_channel::SendError(v));
                }
                Err(TrySendError::Full(v)) => {
                    value = v;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }
}
impl<T> Receiver<T> {
    pub(crate) fn recovery(&self) -> &Recovery {
        &self.1
    }
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        self.0.try_iter().map(|e| {
            self.1.consumed(e.queued_qpc);
            e.value
        })
    }
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.0.try_recv().map(|e| {
            self.1.consumed(e.queued_qpc);
            e.value
        })
    }
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Result<T, RecvTimeoutError> {
        self.0.recv_timeout(timeout).map(|e| {
            self.1.consumed(e.queued_qpc);
            e.value
        })
    }
    pub fn recv(&self) -> Result<T, crossbeam_channel::RecvError> {
        self.0.recv().map(|e| {
            self.1.consumed(e.queued_qpc);
            e.value
        })
    }
}

struct AsyncEntry<T> {
    queued_qpc: u64,
    value: T,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
pub(crate) struct AsyncSender<T> {
    recovery: Arc<Recovery>,
    tx: tokio::sync::mpsc::Sender<AsyncEntry<T>>,
    admission: Arc<tokio::sync::Semaphore>,
}
pub(crate) struct AsyncReceiver<T>(tokio::sync::mpsc::Receiver<AsyncEntry<T>>, Arc<Recovery>);
impl<T> Clone for AsyncSender<T> {
    fn clone(&self) -> Self {
        Self {
            recovery: self.recovery.clone(),
            tx: self.tx.clone(),
            admission: self.admission.clone(),
        }
    }
}
#[track_caller]
pub(crate) fn async_channel<T>(capacity: usize) -> (AsyncSender<T>, AsyncReceiver<T>) {
    let (tx, rx) = tokio::sync::mpsc::channel(capacity);
    let recovery = Recovery::new(capacity);
    (
        AsyncSender {
            recovery: recovery.clone(),
            tx,
            admission: Arc::new(tokio::sync::Semaphore::new(
                capacity - crate::realtime::reserve(capacity),
            )),
        },
        AsyncReceiver(rx, recovery),
    )
}
impl<T: Priority> AsyncSender<T> {
    pub(crate) fn recovery(&self) -> &Recovery {
        &self.recovery
    }
    pub fn try_send(&self, value: T) -> Result<(), tokio::sync::mpsc::error::TrySendError<T>> {
        use tokio::sync::mpsc::error::TrySendError;
        let critical = value.critical();
        let permit = if critical {
            None
        } else {
            match self.admission.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => {
                    self.recovery.rejected(false);
                    self.recovery.dropped(false);
                    return Err(TrySendError::Full(value));
                }
            }
        };
        self.tx
            .try_send(AsyncEntry {
                queued_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                value,
                _permit: permit,
            })
            .map(|()| {
                self.recovery
                    .admitted(self.tx.max_capacity() - self.tx.capacity())
            })
            .map_err(|e| {
                self.recovery.rejected(critical);
                self.recovery.dropped(critical);
                match e {
                    TrySendError::Full(e) => TrySendError::Full(e.value),
                    TrySendError::Closed(e) => TrySendError::Closed(e.value),
                }
            })
    }
    #[cfg(test)]
    pub(crate) async fn send(
        &self,
        value: T,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        let critical = value.critical();
        let permit = if critical {
            None
        } else {
            self.admission.clone().acquire_owned().await.ok()
        };
        self.tx
            .send(AsyncEntry {
                queued_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                value,
                _permit: permit,
            })
            .await
            .map(|()| {
                self.recovery
                    .admitted(self.tx.max_capacity() - self.tx.capacity())
            })
            .map_err(|e| tokio::sync::mpsc::error::SendError(e.0.value))
    }
}
impl<T> AsyncReceiver<T> {
    pub(crate) fn recovery(&self) -> &Recovery {
        &self.1
    }
    pub(crate) async fn recv(&mut self) -> Option<T> {
        self.0.recv().await.map(|e| {
            self.1.consumed(e.queued_qpc);
            e.value
        })
    }
    pub fn try_recv(&mut self) -> Result<T, tokio::sync::mpsc::error::TryRecvError> {
        self.0.try_recv().map(|e| {
            self.1.consumed(e.queued_qpc);
            e.value
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generation_change_preserves_new_fifo_events_until_reset() {
        // Either the common input or just this destination may invalidate work
        // between a reader's observation and its dequeue.
        for current in [(1, 0), (0, 1), (1, 1)] {
            assert_eq!(
                generation_action((0, 0), (0, 0), current),
                GenerationAction::Discard
            );
            assert_eq!(
                generation_action(current, (0, 0), current),
                GenerationAction::Defer
            );
            assert_eq!(
                generation_action(current, current, current),
                GenerationAction::Ready
            );
        }
        let (tx, rx) = bounded(8);
        let mut old = note(0x90, 60);
        old.generation = 0;
        tx.try_send(old).unwrap();
        for status in [0x90, 0x80] {
            let mut fresh = note(status, 61);
            fresh.generation = 1;
            tx.try_send(fresh).unwrap();
        }
        assert_eq!(
            generation_action((rx.recv().unwrap().generation, 0), (0, 0), (1, 0)),
            GenerationAction::Discard
        );
        let pending = rx.recv().unwrap();
        assert_eq!(
            generation_action((pending.generation, 0), (0, 0), (1, 0)),
            GenerationAction::Defer
        );
        // Emit the reset before accepting this saved new event, then continue FIFO.
        assert_eq!(
            generation_action((pending.generation, 0), (1, 0), (1, 0)),
            GenerationAction::Ready
        );
        assert_eq!(pending.data[0], 0x90);
        assert_eq!(rx.recv().unwrap().data[0], 0x80);
    }
    fn note(status: u8, key: u8) -> crate::midi::MidiFrame {
        crate::midi::MidiFrame::copy_realtime(&[status, key, 100], Arc::from("test")).unwrap()
    }
    #[test]
    fn successful_release_retry_counts_rejection_without_a_drop() {
        let (tx, rx) = bounded(1);
        tx.try_send(note(0x90, 60)).unwrap();
        let release = tx
            .try_send_retryable(note(0x80, 60))
            .unwrap_err()
            .into_inner();
        assert_eq!(tx.recovery().snapshot().critical_rejections, 1);
        assert_eq!(tx.recovery().snapshot().critical_drops, 0);
        rx.recv().unwrap();
        tx.try_send_retryable(release).unwrap();
        assert_eq!(rx.recv().unwrap().data[0], 0x80);
        assert_eq!(tx.recovery().snapshot().critical_drops, 0);
    }

    #[test]
    fn pedal_and_channel_mode_releases_use_reserved_capacity() {
        for controller in [64, 66, 69, 120, 121, 123, 124, 125, 126, 127] {
            let (tx, rx) = bounded(8);
            for key in 0..7 {
                tx.try_send(note(0x90, key)).unwrap();
            }
            let release =
                crate::midi::MidiFrame::copy_realtime(&[0xB0, controller, 0], Arc::from("test"))
                    .unwrap();
            assert!(tx.try_send(release).is_ok(), "CC {controller} release lost");
            assert_eq!(
                rx.try_iter().last().unwrap().data.as_slice(),
                &[0xB0, controller, 0]
            );
        }
        for controller in [64, 66, 69] {
            assert!(!crate::midi::is_critical_release_message(&[
                0xB0, controller, 64
            ]));
            assert!(crate::midi::is_critical_release_message(&[
                0xB0, controller, 63
            ]));
        }
        assert!(!crate::midi::is_critical_release_message(&[0xB0, 122, 0]));
    }

    #[test]
    fn reserve_preserves_fifo_and_returns_admission_on_dequeue() {
        let (tx, rx) = bounded(8);
        for key in 0..7 {
            tx.try_send(note(0x90, key)).unwrap();
        }
        assert!(tx.try_send(note(0x90, 8)).is_err());
        tx.try_send(note(0x80, 0)).unwrap();
        assert_eq!(rx.recv().unwrap().data[1], 0);
        tx.try_send(note(0x90, 9)).unwrap();
        for key in 1..7 {
            assert_eq!(rx.recv().unwrap().data[1], key);
        }
        assert_eq!(rx.recv().unwrap().data[0], 0x80);
        assert_eq!(rx.recv().unwrap().data[1], 9);
    }
    #[tokio::test]
    async fn async_reserve_preserves_fifo_and_releases_failed_admission() {
        let (tx, mut rx) = async_channel(8);
        for key in 0..7 {
            tx.try_send(note(0x90, key)).unwrap();
        }
        assert!(tx.try_send(note(0x90, 8)).is_err());
        tx.try_send(note(0x80, 0)).unwrap();
        assert!(tx.try_send(note(0x80, 1)).is_err());
        for key in 0..7 {
            assert_eq!(rx.recv().await.unwrap().data[1], key);
        }
        assert_eq!(rx.recv().await.unwrap().data[0], 0x80);
        tx.try_send(note(0x90, 8)).unwrap();
    }
}
