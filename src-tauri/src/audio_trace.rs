//! Opt-in per-block evidence. Initialized before audio; bounded, allocation-free
//! bounded SPSC push behind try_lock on realtime threads; JSON and disk access
//! only on the recorder thread. Contention drops a record without waiting.
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        LazyLock,
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Serialize)]
pub enum Stage {
    Callback,
    BridgeRequest,
    BridgeConsumed,
    BridgeDiscarded,
    BridgeMissing,
    DspBegin,
    DspEnd,
    PowerPolicy,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BlockTrace {
    pub stage: Stage,
    pub qpc: u64,
    pub sequence: u64,
    pub frames: u32,
    /// Expected sequence on discard; cumulative xrun count on callback;
    /// MIDI event count on request/DSP; otherwise zero.
    pub detail: u64,
    pub duration_us: u64,
    pub peak_l: f32,
    pub peak_r: f32,
}

struct Buffer {
    tx: Mutex<rtrb::Producer<BlockTrace>>,
    rx: Mutex<rtrb::Consumer<BlockTrace>>,
    dropped: AtomicU64,
}
impl Buffer {
    fn new(capacity: usize) -> Self {
        let (tx, rx) = rtrb::RingBuffer::new(capacity);
        Self {
            tx: Mutex::new(tx),
            rx: Mutex::new(rx),
            dropped: AtomicU64::new(0),
        }
    }
    fn push(&self, row: BlockTrace) {
        if self
            .tx
            .try_lock()
            .is_none_or(|mut tx| tx.push(row).is_err())
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
static BUFFER: LazyLock<Buffer> = LazyLock::new(|| Buffer::new(8192));
static ENABLED: AtomicBool = AtomicBool::new(false);
type FinishChannels = (Sender<()>, crossbeam_channel::Receiver<bool>);
static FINISH: Mutex<Option<FinishChannels>> = Mutex::new(None);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn record(row: BlockTrace) {
    if enabled() {
        BUFFER.push(row);
    }
}

/// Control-thread checkpoints only. sequence: 1=before Tauri, 2=after Tauri,
/// 3=audio tuning applied, 4=stream built, 5=stream playing.
#[cfg(windows)]
pub fn record_power_policy(phase: u64) {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessInformation, ProcessPowerThrottling,
        PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_STATE,
    };
    if !enabled() {
        return;
    }
    let mut power = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..Default::default()
    };
    if unsafe {
        GetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &mut power as *mut _ as _,
            std::mem::size_of_val(&power) as u32,
        )
    }
    .is_ok()
    {
        record(BlockTrace {
            stage: Stage::PowerPolicy,
            qpc: crate::vst_worker::protocol::monotonic_qpc(),
            sequence: phase,
            frames: 0,
            detail: (u64::from(power.ControlMask) << 32) | u64::from(power.StateMask),
            duration_us: 0,
            peak_l: 0.0,
            peak_r: 0.0,
        });
    }
}

pub struct Recorder {
    stop: Sender<()>,
    thread: Option<std::thread::JoinHandle<io::Result<()>>>,
}

/// Stop/ack happens on the control path before the supervisor may kill the
/// worker after its Stopped reply. Never call this from an audio callback.
pub fn finish_file_recorder() -> io::Result<()> {
    ENABLED.store(false, Ordering::Release);
    if let Some((stop, done)) = FINISH.lock().take() {
        let _ = stop.try_send(());
        if done.recv_timeout(Duration::from_secs(2)) != Ok(true) {
            return Err(io::Error::other(
                "Audio trace finalization failed or timed out",
            ));
        }
    }
    Ok(())
}
impl Drop for Recorder {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::Release);
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            if !matches!(thread.join(), Ok(Ok(()))) {
                eprintln!("Audio trace writer failed; evidence may be incomplete");
            }
        }
    }
}

/// Missing environment variable leaves tracing disabled. Each process creates
/// its own file, with QPC frequency and a final overflow counter for audit.
pub fn start_file_recorder() -> io::Result<Option<Recorder>> {
    let Some(dir) = std::env::var_os("OSCMIDI_AUDIO_TRACE_DIR") else {
        return Ok(None);
    };
    let dir = PathBuf::from(dir);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(io::Error::other(
            "OSCMIDI_AUDIO_TRACE_DIR must be an existing absolute directory",
        ));
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(format!(
            "audio-{}-{}.jsonl",
            std::process::id(),
            usize::BITS
        )))?;
    // Force the allocation on this control thread, before any callback can run.
    let _ = &*BUFFER;
    let (stop, rx) = crossbeam_channel::bounded(1);
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);
    let thread=std::thread::Builder::new().name("audio-trace-writer".into()).spawn(move || {
        let result = (|| -> io::Result<()> {
        let mut writer=BufWriter::new(file);
        serde_json::to_writer(&mut writer,&serde_json::json!({"schema":1,"pid":std::process::id(),"pointerBits":usize::BITS,"qpcFrequency":crate::vst_worker::protocol::qpc_frequency()}))?;
        writeln!(writer)?;
        loop {
            let stop= !matches!(rx.recv_timeout(Duration::from_millis(50)),Err(crossbeam_channel::RecvTimeoutError::Timeout));
            let mut consumer=BUFFER.rx.lock();
            while let Ok(row)=consumer.pop() { serde_json::to_writer(&mut writer,&row)?;writeln!(writer)?; }
            writer.flush()?;
            if stop { break; }
        }
        serde_json::to_writer(&mut writer,&serde_json::json!({"end":true,"dropped":BUFFER.dropped.load(Ordering::Relaxed)}))?;
        writeln!(writer)?;writer.flush()
        })();
        let _ = done_tx.try_send(result.is_ok());
        result
    })?;
    *FINISH.lock() = Some((stop.clone(), done_rx));
    ENABLED.store(true, Ordering::Release);
    Ok(Some(Recorder {
        stop,
        thread: Some(thread),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_trace_queue_never_allocates_or_waits_and_counts_loss() {
        let buffer = Buffer::new(2);
        let row = BlockTrace {
            stage: Stage::BridgeMissing,
            qpc: 17,
            sequence: 42,
            frames: 512,
            detail: 0,
            duration_us: 0,
            peak_l: 0.0,
            peak_r: 0.0,
        };
        assert_eq!(
            crate::test_alloc::count_allocations(|| {
                for _ in 0..1000 {
                    buffer.push(row);
                }
            }),
            0
        );
        assert_eq!(buffer.dropped.load(Ordering::Relaxed), 998);
        let mut consumer = buffer.rx.lock();
        assert_eq!(consumer.pop().unwrap().sequence, 42);
        assert_eq!(consumer.pop().unwrap().qpc, 17);
        assert!(consumer.pop().is_err());
        let _producer = buffer.tx.lock();
        buffer.push(row);
        assert_eq!(buffer.dropped.load(Ordering::Relaxed), 999);
    }
}
