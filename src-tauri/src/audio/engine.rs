#![allow(deprecated)]

use super::runtime_state::{AudioLifecycleState, AudioRuntime, AudioTelemetry, MidiPacket};
#[cfg(test)]
use super::{
    callback::{midi_to_rack_event, replay_last_output_or_silence, reset_messages_for_channel},
    plugin_host::SimpleHost,
    runtime_state::{
        AudioCallbackState, AudioControls, MAX_PENDING_MIDI, MIDI_DRAIN_BUDGET_PER_CALLBACK,
    },
    state_codec::{decode_state, encode_state_chunk, encode_state_params, SavedState},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
};

use arc_swap::ArcSwapOption;
use parking_lot::Mutex;
#[cfg(test)]
use rack::prelude::MidiEventKind as RackMidiEventKind;
#[cfg(test)]
use rack::prelude::PluginScanner as _;
#[cfg(test)]
use rack::vst3::Vst3Scanner;
#[cfg(test)]
use rack::PluginInstance as _;
use rtrb::Producer;
#[cfg(test)]
use std::sync::atomic::AtomicU32;
#[cfg(test)]
use vst::host::PluginLoader;
#[cfg(test)]
use vst::plugin::Plugin;

#[cfg(test)]
use super::stream_config::{buffer_fallback_candidates, choose_buffer_size, max_plugin_block_size};

#[cfg(test)]
use crate::plugin_probe::detect_vst3_channels;

pub(super) struct MidiIngress {
    producer: Mutex<Producer<MidiPacket>>,
    telemetry: Arc<AudioTelemetry>,
    reserve: usize,
}
impl MidiIngress {
    pub(super) fn new(producer: Producer<MidiPacket>, telemetry: Arc<AudioTelemetry>) -> Self {
        let reserve = crate::realtime::reserve(producer.slots());
        Self {
            reserve,
            producer: Mutex::new(producer),
            telemetry,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MidiSendOutcome {
    Sent,
    DroppedNonCritical,
    CriticalFallbackReset,
    AudioStopped,
}

#[derive(Clone)]
pub struct AudioEngine {
    pub(super) midi_transport_failed: Arc<AtomicBool>,
    pub(super) live_parameters: Arc<Mutex<std::collections::BTreeMap<usize, f32>>>,
    pub(super) game_mode: Arc<AtomicBool>,
    pub(super) editor_open: Arc<AtomicBool>,
    pub(super) maintenance: Arc<AtomicBool>,
    pub(super) deferred_save: Arc<AtomicBool>,
    pub(super) deferred_editor: Arc<AtomicU8>,
    pub(super) health_telemetry: Arc<ArcSwapOption<AudioTelemetry>>,
    pub(super) lifecycle_gate: Arc<Mutex<()>>,
    pub(super) lifecycle_state: Arc<AtomicU8>,
    pub(super) runtime: Arc<Mutex<Option<AudioRuntime>>>,
    pub(super) last_vst: Arc<Mutex<Option<PathBuf>>>,
    pub(super) midi_tx: Arc<ArcSwapOption<MidiIngress>>,
    pub(super) midi_emergency_reset_requested: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    pub(super) worker_enabled: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    pub(super) worker: crate::vst_worker::VstWorkerSupervisor,
}

impl AudioEngine {
    pub fn new() -> Self {
        super::thread_affinity::register_main_ui_thread();
        Self {
            midi_transport_failed: Arc::new(AtomicBool::new(false)),
            live_parameters: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            game_mode: Arc::new(AtomicBool::new(true)),
            editor_open: Arc::new(AtomicBool::new(false)),
            maintenance: Arc::new(AtomicBool::new(false)),
            deferred_save: Arc::new(AtomicBool::new(false)),
            deferred_editor: Arc::new(AtomicU8::new(0)),
            health_telemetry: Arc::new(ArcSwapOption::empty()),
            lifecycle_gate: Arc::new(Mutex::new(())),
            lifecycle_state: Arc::new(AtomicU8::new(AudioLifecycleState::Stopped as u8)),
            runtime: Arc::new(Mutex::new(None)),
            last_vst: Arc::new(Mutex::new(None)),
            midi_tx: Arc::new(ArcSwapOption::empty()),
            midi_emergency_reset_requested: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "windows")]
            worker_enabled: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "windows")]
            worker: crate::vst_worker::VstWorkerSupervisor::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.is_ready();
        }
        self.runtime.lock().is_some()
    }

    pub fn is_vst_loaded(&self) -> bool {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.is_ready();
        }
        self.runtime.lock().is_some()
    }

    pub fn send_midi(&self, bytes: &[u8]) {
        let _ = self.send_midi_with_outcome(bytes);
    }

    /// Requests a panic without blocking the bridge or an input callback.
    pub fn request_emergency_midi_reset(&self) {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            self.worker.request_emergency_reset();
            return;
        }
        if let Some(ingress) = self.midi_tx.load().as_ref() {
            ingress
                .telemetry
                .midi_generation
                .fetch_add(1, Ordering::AcqRel);
            return;
        }
        self.midi_emergency_reset_requested
            .store(true, Ordering::Release);
    }

    pub fn send_midi_with_age(&self, bytes: &[u8], age_us: u64) {
        let Some(packet) = MidiPacket::from_bytes_with_age(bytes, age_us) else {
            return;
        };
        let _ = self.send_midi_packet_with_outcome(packet);
    }

    pub(crate) fn send_midi_with_outcome(&self, bytes: &[u8]) -> MidiSendOutcome {
        self.send_midi_at(bytes, crate::vst_worker::protocol::monotonic_qpc())
    }

    pub(crate) fn send_midi_at(&self, bytes: &[u8], ingress_qpc: u64) -> MidiSendOutcome {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return if self.worker.try_send_midi_at(bytes, ingress_qpc) {
                MidiSendOutcome::Sent
            } else if crate::midi::is_critical_release_message(bytes) {
                self.worker.request_emergency_reset();
                MidiSendOutcome::CriticalFallbackReset
            } else {
                MidiSendOutcome::DroppedNonCritical
            };
        }

        let Some(packet) = MidiPacket::from_bytes(bytes) else {
            return MidiSendOutcome::DroppedNonCritical;
        };

        let mut packet = packet;
        packet.ingress_qpc = ingress_qpc;
        self.send_midi_packet_with_outcome(packet)
    }

    pub fn report_midi_transport_failure(&self) {
        self.midi_transport_failed.store(true, Ordering::Release);
    }

    pub fn deliver_midi_wire(&self, frame: crate::vst_worker::MidiWireFrame) {
        if frame.pipe_qpc != 0 {
            if let Some(ingress) = self.midi_tx.load().as_ref() {
                ingress.telemetry.pipe_wait_max_us.fetch_max(
                    crate::vst_worker::protocol::qpc_elapsed_us(
                        frame.pipe_qpc,
                        crate::vst_worker::protocol::monotonic_qpc(),
                    ),
                    Ordering::Relaxed,
                );
            }
        }
        let age = crate::vst_worker::protocol::qpc_elapsed_us(
            frame.monotonic_qpc,
            crate::vst_worker::protocol::monotonic_qpc(),
        );
        if let Some(mut packet) = MidiPacket::from_bytes_with_age(frame.bytes(), age) {
            packet.ingress_qpc = frame.monotonic_qpc;
            let _ = self.send_midi_packet_with_outcome(packet);
        }
    }

    fn send_midi_packet_with_outcome(&self, mut packet: MidiPacket) -> MidiSendOutcome {
        let route = self.midi_tx.load();
        let Some(ingress) = route.as_ref() else {
            return MidiSendOutcome::AudioStopped;
        };
        packet.generation = ingress.telemetry.midi_generation.load(Ordering::Acquire);
        if ingress.producer.try_lock().is_some_and(|mut producer| {
            if !packet.is_critical_release() && producer.slots() <= ingress.reserve {
                return false;
            }
            producer.push(packet).is_ok()
        }) {
            return MidiSendOutcome::Sent;
        }
        ingress
            .telemetry
            .midi_drop_count
            .fetch_add(1, Ordering::Relaxed);
        if packet.is_critical_release() {
            self.request_emergency_midi_reset();
            MidiSendOutcome::CriticalFallbackReset
        } else {
            MidiSendOutcome::DroppedNonCritical
        }
    }

    #[cfg(target_os = "windows")]
    pub(super) fn is_worker_enabled(&self) -> bool {
        self.worker_enabled.load(Ordering::Acquire)
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn is_sforzando_vst3(vst_path: &Path) -> bool {
    let path = vst_path.to_string_lossy().to_lowercase();
    path.ends_with("sforzando.vst3") || path.contains("\\sforzando.vst3")
}

const UPRIGHT_PIANO_VST3_CLASS_UID: &str = "5653545F474B64757072696768742070";

pub(crate) fn requires_vst3_destructor_quarantine(
    vst_path: &Path,
    class_uid: Option<&str>,
) -> bool {
    if !vst_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vst3"))
    {
        return false;
    }

    // Upright Piano completes initialization and audio processing, but the
    // currently distributed Windows VST3 raises an access violation from its
    // native destructor. Identify it by its VST3 class when available; retain
    // the filename fallback for migrated caches created before class IDs were
    // persisted.
    if class_uid.is_some_and(|uid| uid.eq_ignore_ascii_case(UPRIGHT_PIANO_VST3_CLASS_UID)) {
        return true;
    }

    if is_sforzando_vst3(vst_path) {
        return true;
    }
    let path = vst_path.to_string_lossy().to_lowercase();
    path.contains("\\splice\\")
        || path.ends_with("splice instrument.vst3")
        || path.ends_with("upright piano.vst3")
}

pub(super) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use rack::prelude::PluginType as RackPluginType;
    use rtrb::RingBuffer;
    use std::{thread, time::Instant};

    fn callback_test_state(
        rx: rtrb::Consumer<MidiPacket>,
        emergency_reset_requested: Arc<AtomicBool>,
    ) -> AudioCallbackState {
        AudioCallbackState::new(
            rx,
            0,
            2,
            1024,
            2,
            Arc::new(AudioControls {
                gain_bits: AtomicU32::new(1.0f32.to_bits()),
                limiter_enabled: AtomicBool::new(false),
                equalizer: crate::audio::equalizer::PublishedEq::new(
                    &crate::config::VstEqSettings::default(),
                    48_000,
                ),
            }),
            Arc::new(AudioTelemetry::new()),
            emergency_reset_requested,
            48_000,
            crossbeam_channel::bounded(8).1,
        )
    }

    #[test]
    fn vst3_reset_batch_preserves_notes_across_capacity_boundary() {
        use super::super::callback_midi::submit_pending_vst3_midi;

        for count in [448, 449, 512] {
            let (_, rx) = RingBuffer::new(1024);
            let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
            let mut expected = Vec::new();
            for index in 0..count {
                let bytes = if index == count - 2 {
                    [0x80, 60, 0]
                } else if index == count - 1 {
                    [0x90, 60, 100]
                } else {
                    [0xB0, 1, (index % 128) as u8]
                };
                state
                    .pending_midi
                    .push_back(MidiPacket::from_bytes(&bytes).unwrap());
                expected.push(midi_to_rack_event(bytes).unwrap().kind);
            }
            let scratch_capacity = state.midi_events.capacity();
            let pending_capacity = state.pending_midi.capacity();
            let resets = (0..16)
                .flat_map(reset_messages_for_channel)
                .map(|bytes| midi_to_rack_event(bytes).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(resets.len(), 64);
            let mut backend = Vec::with_capacity(512);
            backend.extend_from_slice(&resets);
            submit_pending_vst3_midi(&mut state, 512 - backend.len(), |events| {
                assert!(backend.len() + events.len() <= backend.capacity());
                backend.extend_from_slice(events);
                Ok(())
            })
            .unwrap();
            assert_eq!(&backend[..64], resets.as_slice());
            assert_eq!(backend.len(), 512);
            assert_eq!(state.pending_midi.len(), count - 448);
            let mut delivered = backend[64..]
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>();
            // Processing the first block frees backend capacity for the next.
            backend.clear();
            submit_pending_vst3_midi(&mut state, 512, |events| {
                backend.extend_from_slice(events);
                Ok(())
            })
            .unwrap();
            delivered.extend(backend.iter().map(|event| event.kind));
            assert_eq!(delivered, expected, "FIFO order changed at count {count}");
            assert!(state.pending_midi.is_empty());
            assert_eq!(state.midi_events.capacity(), scratch_capacity);
            assert_eq!(state.pending_midi.capacity(), pending_capacity);
        }
    }

    #[test]
    fn vst3_failed_admission_preserves_note_off_and_retrigger_for_retry() {
        use super::super::callback_midi::submit_pending_vst3_midi;

        let (_, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        for bytes in [[0x80, 60, 0], [0x90, 60, 100]] {
            state
                .pending_midi
                .push_back(MidiPacket::from_bytes(&bytes).unwrap());
        }
        assert!(submit_pending_vst3_midi(&mut state, 512, |_| {
            Err(rack::Error::Other("admission rejected".into()))
        })
        .is_err());
        assert_eq!(state.pending_midi.len(), 2);
        assert!(state.midi_events.is_empty());
        submit_pending_vst3_midi(&mut state, 512, |events| {
            assert_eq!(
                events[0].kind,
                midi_to_rack_event([0x80, 60, 0]).unwrap().kind
            );
            assert_eq!(
                events[1].kind,
                midi_to_rack_event([0x90, 60, 100]).unwrap().kind
            );
            Ok(())
        })
        .unwrap();
        assert!(state.pending_midi.is_empty());
    }

    #[test]
    fn vst3_full_backend_preserves_pending_notes_and_allows_dsp_to_run() {
        use super::super::callback_midi::submit_pending_vst3_midi;

        let (_, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        state
            .pending_midi
            .push_back(MidiPacket::from_bytes(&[0x80, 60, 0]).unwrap());
        submit_pending_vst3_midi(&mut state, 0, |_| {
            panic!("a full backend must not receive another batch")
        })
        .unwrap();
        assert_eq!(state.pending_midi.len(), 1);
        assert!(state.midi_events.is_empty());
    }

    #[test]
    fn midi_to_rack_event_handles_zero_velocity_note_on() {
        let event = midi_to_rack_event([0x90, 60, 0]).expect("event");
        assert!(matches!(
            event.kind,
            RackMidiEventKind::NoteOff {
                note: 60,
                velocity: 0,
                channel: 0
            }
        ));
    }

    #[test]
    fn state_codec_round_trip_chunk() {
        let payload = vec![1u8, 2, 3, 4, 5];
        let encoded = encode_state_chunk(&payload);
        match decode_state(&encoded) {
            SavedState::Chunk(data) => assert_eq!(data, payload),
            _ => panic!("Expected chunk payload"),
        }
    }

    #[test]
    fn state_codec_round_trip_params() {
        let params = vec![0.0f32, 0.5, 1.0];
        let encoded = encode_state_params(&params);
        match decode_state(&encoded) {
            SavedState::Params(data) => assert_eq!(data, params),
            _ => panic!("Expected params payload"),
        }
    }

    #[test]
    fn state_codec_raw_when_no_header() {
        let payload = vec![9u8, 8, 7];
        match decode_state(&payload) {
            SavedState::Raw(data) => assert_eq!(data, payload),
            _ => panic!("Expected raw payload"),
        }
    }

    #[test]
    fn reset_sequence_includes_sustain_and_panic_controllers() {
        let messages = reset_messages_for_channel(2);
        assert_eq!(
            messages,
            [
                [0xB2, 64, 0],
                [0xB2, 120, 0],
                [0xB2, 121, 0],
                [0xB2, 123, 0],
            ]
        );
    }

    #[test]
    fn emergency_reset_clears_pending_midi() {
        let (mut tx, rx) = RingBuffer::new(8);
        tx.push(MidiPacket::from_bytes(&[0x90, 60, 100]).unwrap())
            .unwrap();
        let reset_flag = Arc::new(AtomicBool::new(true));
        let mut state = callback_test_state(rx, reset_flag);
        state.telemetry.midi_generation.store(1, Ordering::Release);
        // Pre-fill pending MIDI with note-on events
        for note in 0..10u8 {
            state.pending_midi.push_back(MidiPacket {
                worker_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                generation: 0,
                ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                data: [0x90, note, 100],
                len: 3,
                timestamp_us: 0,
            });
        }
        assert_eq!(state.pending_midi.len(), 10);

        // drain_midi should clear pending_midi when emergency_reset_requested is set
        state.drain_midi();

        assert!(
            state.needs_emergency_reset,
            "emergency reset flag should be set"
        );
        assert_eq!(
            state.pending_midi.len(),
            0,
            "pending MIDI must be cleared on emergency reset"
        );
        assert_eq!(
            state.midi_rx.slots(),
            0,
            "Queued notes must not replay after reset"
        );
    }

    #[test]
    fn overflow_prefers_dropping_note_on_for_critical_release() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        for _ in 0..MAX_PENDING_MIDI {
            state
                .pending_midi
                .push_back(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("note on"));
        }

        state.enqueue_midi(MidiPacket::from_bytes(&[0x80, 60, 0]).expect("note off"));

        assert_eq!(state.pending_midi.len(), MAX_PENDING_MIDI);
        assert!(state.pending_midi.iter().any(|m| m.is_note_off()));
        assert!(!state.needs_emergency_reset);
    }

    #[test]
    fn overflow_eviction_preserves_release_and_sustain_order() {
        for candidate in [[0x90, 61, 100], [0xB0, 10, 64]] {
            let (_tx, rx) = RingBuffer::new(8);
            let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
            let second = if candidate[0] == 0x90 {
                [0xB0, 64, 127]
            } else {
                [0x80, 61, 0]
            };
            for data in [[0x80, 60, 0], second, candidate] {
                state
                    .pending_midi
                    .push_back(MidiPacket::from_bytes(&data).unwrap());
            }
            while state.pending_midi.len() < MAX_PENDING_MIDI {
                state
                    .pending_midi
                    .push_back(MidiPacket::from_bytes(&[0x80, 62, 0]).unwrap());
            }
            let capacity = state.pending_midi.capacity();
            state.enqueue_midi(MidiPacket::from_bytes(&[0x80, 63, 0]).unwrap());
            assert_eq!(state.pending_midi.len(), MAX_PENDING_MIDI);
            assert_eq!(state.pending_midi.capacity(), capacity);
            assert_eq!(state.pending_midi[0].data, [0x80, 60, 0]);
            assert_eq!(state.pending_midi[1].data, second);
            assert_eq!(state.pending_midi.back().unwrap().data, [0x80, 63, 0]);
        }
    }

    #[test]
    fn reset_invalidates_old_ring_events_but_keeps_new_fifo_events() {
        let engine = AudioEngine::new();
        let (tx, rx) = RingBuffer::new(32);
        let mut callback = callback_test_state(rx, engine.midi_emergency_reset_requested.clone());
        let ingress = Arc::new(MidiIngress::new(tx, callback.telemetry.clone()));
        engine.midi_tx.store(Some(ingress));
        engine.send_midi(&[0x90, 60, 100]);
        engine.request_emergency_midi_reset();
        engine.send_midi(&[0x90, 61, 100]);
        engine.send_midi(&[0x80, 61, 0]);
        callback.drain_midi();
        assert!(callback.needs_emergency_reset);
        assert_eq!(callback.pending_midi.len(), 2);
        assert_eq!(callback.pending_midi[0].data, [0x90, 61, 100]);
        assert_eq!(callback.pending_midi[1].data, [0x80, 61, 0]);
    }

    #[test]
    fn duplicate_parameters_count_toward_callback_budget() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut callback = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        let (tx, rx) = crossbeam_channel::bounded(1024);
        callback.parameter_rx = rx;
        for sequence in 1..=1024 {
            tx.try_send(super::super::runtime_state::ParameterCommand {
                sequence,
                index: 0,
                id: 0,
                flags: 0,
                step_count: 0,
                value: 0.5,
            })
            .unwrap();
        }
        callback.drain_parameter_commands();
        assert_eq!(callback.parameter_commands.len(), 1);
        assert_eq!(callback.parameter_rx.len(), 512);
        assert_eq!(callback.parameter_commands[0].sequence, 512);
        assert_eq!(
            callback
                .telemetry
                .parameter_applied_sequence
                .load(Ordering::Acquire),
            0
        );
        super::super::callback::acknowledge_parameters(&callback);
        assert_eq!(
            callback
                .telemetry
                .parameter_applied_sequence
                .load(Ordering::Acquire),
            512
        );
        callback.drain_parameter_commands_limit(0);
        assert!(callback.parameter_commands.is_empty());
        assert_eq!(callback.parameter_rx.len(), 512);
    }

    #[test]
    fn later_success_cannot_hide_a_failed_parameter_application() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut callback = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        callback
            .parameter_commands
            .push(super::super::runtime_state::ParameterCommand {
                sequence: 1,
                index: 0,
                id: 0,
                flags: 0,
                step_count: 0,
                value: 0.25,
            });
        super::super::callback::fail_parameters(&callback);
        callback.parameter_commands[0].sequence = 2;
        callback.parameter_commands[0].value = 0.75;
        super::super::callback::acknowledge_parameters(&callback);
        callback
            .telemetry
            .parameter_accepted_sequence
            .store(2, Ordering::Release);
        assert_eq!(
            callback
                .telemetry
                .parameter_applied_sequence
                .load(Ordering::Acquire),
            2
        );
        assert!(super::super::game_mode::wait_for_parameters(
            &callback.telemetry,
            std::time::Duration::ZERO
        )
        .is_err());
    }

    #[test]
    fn heartbeat_snapshot_does_not_wait_for_host_runtime_lock() {
        let engine = AudioEngine::new();
        let telemetry = Arc::new(AudioTelemetry::new());
        telemetry.callback_count.store(123, Ordering::Relaxed);
        engine.health_telemetry.store(Some(telemetry));
        let _runtime = engine.runtime.lock();
        assert_eq!(engine.realtime_metrics().callback_count, 123);
    }

    #[test]
    fn audio_packets_reject_truncation_and_normalize_zero_velocity() {
        for bytes in [
            &[][..],
            &[0x90, 60],
            &[0x90, 128, 1],
            &[0x90, 60, 1, 0],
            &[0xF0, 1, 0xF7],
        ] {
            assert!(MidiPacket::from_bytes(bytes).is_none());
        }
        assert_eq!(
            MidiPacket::from_bytes(&[0x91, 60, 0]).unwrap().data,
            [0x81, 60, 0]
        );
        assert_eq!(MidiPacket::from_bytes(&[0xC0, 127]).unwrap().len, 2);
    }

    #[test]
    fn audio_submission_never_waits_for_runtime_or_producer_guards() {
        let engine = AudioEngine::new();
        let (tx, mut rx) = RingBuffer::new(8);
        let telemetry = Arc::new(AudioTelemetry::new());
        let ingress = Arc::new(MidiIngress::new(tx, telemetry.clone()));
        engine.midi_tx.store(Some(ingress.clone()));
        let _runtime = engine.runtime.lock();
        assert_eq!(
            engine.send_midi_with_outcome(&[0x90, 60, 100]),
            MidiSendOutcome::Sent
        );
        assert!(rx.pop().is_ok());
        let _producer = ingress.producer.lock();
        assert_eq!(
            engine.send_midi_with_outcome(&[0x80, 60, 0]),
            MidiSendOutcome::CriticalFallbackReset
        );
        assert_eq!(telemetry.midi_drop_count.load(Ordering::Relaxed), 1);
        assert_eq!(telemetry.midi_generation.load(Ordering::Acquire), 1);
    }

    #[test]
    fn stress_audio_ring_buffer_no_loss() {
        const TOTAL: u64 = 500_000;
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(16_384);

        let producer = thread::spawn(move || {
            for i in 0..TOTAL {
                let packet = MidiPacket {
                    worker_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    generation: 0,
                    ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    data: [0x90, (i % 88) as u8 + 21, 100],
                    len: 3,
                    timestamp_us: i,
                };
                loop {
                    if tx.push(packet).is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        });

        let start = Instant::now();
        let mut received = 0u64;
        let mut next_timestamp = 0u64;
        while received < TOTAL {
            match rx.pop() {
                Ok(packet) => {
                    assert_eq!(
                        packet.timestamp_us, next_timestamp,
                        "audio ring order mismatch"
                    );
                    next_timestamp += 1;
                    received += 1;
                }
                Err(_) => thread::yield_now(),
            }
        }
        let elapsed = start.elapsed();
        producer.join().expect("producer join");
        assert_eq!(received, TOTAL, "audio ring lost midi packets");
        eprintln!(
            "Audio ring stress: {received} packets in {:?} ({:.0} pkt/s)",
            elapsed,
            received as f64 / elapsed.as_secs_f64()
        );
    }

    #[test]
    fn send_midi_drops_non_critical_when_ring_full() {
        let engine = AudioEngine::new();
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(1);
        tx.push(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("first packet"))
            .expect("prefill ring");
        engine.midi_tx.store(Some(Arc::new(MidiIngress::new(
            tx,
            Arc::new(AudioTelemetry::new()),
        ))));

        let outcome = engine.send_midi_with_outcome(&[0x90, 61, 100]);

        assert_eq!(outcome, MidiSendOutcome::DroppedNonCritical);

        let first = rx.pop().expect("first packet still queued");
        assert_eq!(first.data, [0x90, 60, 100]);
        assert!(rx.pop().is_err(), "non-critical note-on must be dropped");
    }

    #[test]
    fn send_midi_preserves_critical_release_or_requests_reset() {
        let engine = AudioEngine::new();
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(1);
        tx.push(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("first packet"))
            .expect("prefill ring");
        engine.midi_tx.store(Some(Arc::new(MidiIngress::new(
            tx,
            Arc::new(AudioTelemetry::new()),
        ))));

        let outcome = engine.send_midi_with_outcome(&[0x80, 60, 0]);

        assert_eq!(outcome, MidiSendOutcome::CriticalFallbackReset);
        assert_eq!(
            engine
                .midi_tx
                .load()
                .as_ref()
                .unwrap()
                .telemetry
                .midi_generation
                .load(Ordering::Acquire),
            1,
            "critical release overflow must invalidate the old admission epoch"
        );

        let first = rx.pop().expect("first packet still queued");
        assert_eq!(first.data, [0x90, 60, 100]);
        assert!(
            rx.pop().is_err(),
            "ring should not receive stale fallback data"
        );
    }

    #[test]
    fn delayed_callback_preserves_note_and_controller_order_without_dropping() {
        let (mut tx, rx) = RingBuffer::<MidiPacket>::new(8);
        let expected = [[0x90, 60, 100], [0xB0, 7, 80], [0x80, 60, 0]];
        for bytes in expected {
            tx.push(MidiPacket::from_bytes(&bytes).unwrap()).unwrap();
        }
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        // Reproduce a wake-up later than two 1024-frame blocks at 48 kHz.
        thread::sleep(std::time::Duration::from_millis(60));
        state.drain_midi();
        assert_eq!(
            state
                .pending_midi
                .iter()
                .map(|packet| packet.data)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(state.telemetry.midi_drop_count.load(Ordering::Relaxed), 0);
        assert!(state.telemetry.audio_midi_oldest_us.load(Ordering::Relaxed) >= 50_000);
    }

    #[test]
    fn drain_midi_respects_callback_budget() {
        let (mut tx, rx) = RingBuffer::<MidiPacket>::new(MIDI_DRAIN_BUDGET_PER_CALLBACK + 128);
        for i in 0..(MIDI_DRAIN_BUDGET_PER_CALLBACK + 64) {
            tx.push(MidiPacket::from_bytes(&[0x90, (i % 100) as u8, 100]).expect("packet"))
                .expect("push packet");
        }

        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));

        state.drain_midi();

        assert_eq!(state.pending_midi.len(), MIDI_DRAIN_BUDGET_PER_CALLBACK);
        let mut remaining = 0usize;
        while state.midi_rx.pop().is_ok() {
            remaining += 1;
        }
        assert_eq!(remaining, 64);
    }

    #[test]
    fn callback_buffers_are_fixed_after_initialization() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        let input_ptr = state.input_silence.as_ptr();
        let output_ptrs: Vec<_> = state.outputs.iter().map(Vec::as_ptr).collect();
        let pending_capacity = state.pending_midi.capacity();

        assert!(state.prepare(1024));
        assert_eq!(state.input_silence.as_ptr(), input_ptr);
        assert_eq!(
            state.outputs.iter().map(Vec::as_ptr).collect::<Vec<_>>(),
            output_ptrs
        );
        assert_eq!(state.pending_midi.capacity(), pending_capacity);
        assert!(!state.prepare(1025));
    }

    #[test]
    fn choose_buffer_size_respects_requested_when_supported() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(choose_buffer_size(&supported, 128), 128);
        assert_eq!(choose_buffer_size(&supported, 32), 64);
        assert_eq!(choose_buffer_size(&supported, 2048), 1024);
        assert_eq!(choose_buffer_size(&supported, 480), 480);
    }

    #[test]
    fn max_plugin_block_size_covers_supported_range() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(max_plugin_block_size(&supported, 480), 1024);
        assert_eq!(max_plugin_block_size(&supported, 2048), 2048);
    }

    #[test]
    fn buffer_fallback_candidates_are_sorted_around_preferred() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        let c = buffer_fallback_candidates(&supported, 128);
        assert_eq!(c.first().copied(), Some(128));
        assert!(c.contains(&256));
        assert!(c.contains(&480));
        assert!(c.contains(&512));
        assert!(!c.contains(&1536));
    }

    #[test]
    fn replay_last_output_ramps_previous_block_to_silence_on_lock_miss() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        state.last_output = vec![0.25, -0.5, 0.1, -0.2];
        let mut out = vec![0.0f32; 4];

        replay_last_output_or_silence(&mut out, 2, 0.0f32, &mut state);

        assert_eq!(out, vec![0.25, -0.375, 0.05, -0.05]);
        assert!(state.last_output.iter().all(|sample| *sample == 0.0));
        assert!(state.recovering_from_silence);
        assert_eq!(
            f32::from_bits(state.telemetry.meter_left.load(Ordering::Relaxed)),
            0.25
        );
        assert_eq!(
            f32::from_bits(state.telemetry.meter_right.load(Ordering::Relaxed)),
            0.375
        );
    }

    #[test]
    fn replay_last_output_falls_back_to_silence_without_history() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        state.last_output.clear();
        let mut out = vec![1.0f32; 4];

        replay_last_output_or_silence(&mut out, 2, 0.0f32, &mut state);

        assert_eq!(out, vec![0.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            f32::from_bits(state.telemetry.meter_left.load(Ordering::Relaxed)),
            0.0
        );
        assert_eq!(
            f32::from_bits(state.telemetry.meter_right.load(Ordering::Relaxed)),
            0.0
        );
    }

    #[test]
    fn quarantines_known_blocking_vst3_destructors_only() {
        assert!(requires_vst3_destructor_quarantine(
            Path::new(r"C:\Program Files\Common Files\VST3\Splice\Splice INSTRUMENT.vst3"),
            None,
        ));
        assert!(requires_vst3_destructor_quarantine(
            Path::new(r"C:\VST3\sforzando.vst3"),
            None,
        ));
        assert!(requires_vst3_destructor_quarantine(
            Path::new(r"D:\Plugins\Renamed Upright.vst3"),
            Some(UPRIGHT_PIANO_VST3_CLASS_UID),
        ));
        assert!(requires_vst3_destructor_quarantine(
            Path::new(r"C:\VST3\Upright Piano.vst3"),
            None,
        ));
        assert!(!requires_vst3_destructor_quarantine(
            Path::new(r"C:\VST3\Other Instrument.vst3"),
            None,
        ));
        assert!(!requires_vst3_destructor_quarantine(
            Path::new(r"C:\VST2\Splice.dll"),
            Some(UPRIGHT_PIANO_VST3_CLASS_UID),
        ));
    }

    #[test]
    #[ignore = "Use the isolated vst_smoke binary for third-party VST2 verification"]
    fn optional_vst2_smoke_test_from_env() {
        let Ok(path) = std::env::var("OSCMIDI_TEST_VST2") else {
            return;
        };

        let vst_path = PathBuf::from(path);
        let host = std::sync::Arc::new(std::sync::Mutex::new(SimpleHost::default()));
        let load_path = crate::plugin_probe::native_plugin_load_path(&vst_path);
        let mut loader =
            PluginLoader::load(&load_path, host).expect("VST2 smoke: failed to load plugin");
        let mut instance = loader
            .instance()
            .expect("VST2 smoke: failed to instantiate plugin");
        instance.init();
        instance.set_sample_rate(48_000.0);
        instance.set_block_size(128);
        instance.resume();

        let info = instance.get_info();
        assert!(info.outputs > 0, "VST2 smoke: plugin has no output buses");
    }

    #[test]
    #[ignore = "Use the isolated vst_smoke binary for third-party VST3 verification"]
    fn optional_vst3_smoke_test_from_env() {
        let Ok(path) = std::env::var("OSCMIDI_TEST_VST3") else {
            return;
        };

        let vst_path = PathBuf::from(path);
        let scanner = Vst3Scanner::new().expect("VST3 smoke: failed to create scanner");
        let scan_root = crate::plugin_probe::vst3_scan_root_for_path(&vst_path);
        let plugins = scanner
            .scan_path(&scan_root)
            .expect("VST3 smoke: failed to scan plugin path");
        let info = crate::plugin_probe::select_vst3_plugin_info_for_path(&plugins, &vst_path)
            .filter(|plugin| {
                matches!(
                    plugin.plugin_type,
                    RackPluginType::Instrument | RackPluginType::Effect
                )
            })
            .expect("VST3 smoke: plugin info not found");
        let mut plugin = scanner
            .load(&info)
            .expect("VST3 smoke: failed to load plugin");
        plugin
            .initialize(48_000.0, 128)
            .expect("VST3 smoke: initialize failed");

        let (input_channels, output_channels) = detect_vst3_channels(&mut plugin, &info, 128)
            .expect("VST3 smoke: channel probe failed");
        eprintln!(
            "VST3 optional smoke channels: inputs={}, outputs={}",
            input_channels, output_channels
        );
    }
}
