use criterion::{black_box, criterion_group, criterion_main, Criterion};
use osc_midi_bridge::{
    audio::AudioEngine, bridge::BridgeHandle, config::ConfigStore, midi::MidiFrame,
};
use smallvec::SmallVec;
use std::sync::Arc;

/// Benchmark du parsing de frames MIDI
fn bench_midi_frame_parsing(c: &mut Criterion) {
    let source: Arc<str> = Arc::from("benchmark");
    let frames: Vec<MidiFrame> = (0..1000)
        .map(|i| {
            let note = (i % 128) as u8;
            let velocity = if i % 2 == 0 { 100 } else { 0 };
            let status = if velocity > 0 { 0x90 } else { 0x80 };

            MidiFrame {
                data: SmallVec::from_slice(&[status, note, velocity]),
                source: Arc::clone(&source),
            }
        })
        .collect();

    c.bench_function("midi_frame_creation", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let note = (i % 128) as u8;
                let velocity = if i % 2 == 0 { 100 } else { 0 };
                let status = if velocity > 0 { 0x90 } else { 0x80 };

                black_box(MidiFrame {
                    data: SmallVec::from_slice(&[status, note, velocity]),
                    source: Arc::clone(&source),
                });
            }
        })
    });

    c.bench_function("midi_frame_clone", |b| {
        b.iter(|| {
            for frame in &frames {
                black_box(frame.clone());
            }
        })
    });
}

/// Benchmark des opérations AudioEngine
fn bench_audio_engine_operations(c: &mut Criterion) {
    let engine = AudioEngine::new();

    c.bench_function("audio_engine_list_backends", |b| {
        b.iter(|| {
            black_box(engine.list_backends());
        })
    });

    let backends = engine.list_backends();
    if let Some(backend) = backends.first() {
        c.bench_function("audio_engine_list_devices", |b| {
            b.iter(|| {
                black_box(engine.list_devices(Some(backend.clone())));
            })
        });
    }

    c.bench_function("audio_engine_metrics", |b| {
        b.iter(|| {
            black_box(engine.xrun_count());
            black_box(engine.midi_drop_count());
            black_box(engine.audio_lock_miss_count());
            black_box(engine.emergency_reset_count());
        })
    });
}

/// Benchmark des opérations Bridge
fn bench_bridge_operations(c: &mut Criterion) {
    let bridge = BridgeHandle::new();
    let config_dir = tempfile::tempdir().expect("benchmark config tempdir");
    let config_store = ConfigStore::with_path(config_dir.path().join("config.yaml"));
    let config = config_store.load();
    config_store.save(&config).expect("seed benchmark config");

    c.bench_function("bridge_status_check", |b| {
        b.iter(|| {
            black_box(bridge.status(&config));
        })
    });

    c.bench_function("config_load_save", |b| {
        b.iter(|| {
            let config = config_store.load();
            let mut modified = config.clone();
            modified.audio.sample_rate = 48_000;
            config_store.save(&modified).unwrap();
            black_box(());
            black_box(config_store.load());
        })
    });
}

/// Benchmark de la sérialisation JSON
fn bench_json_serialization(c: &mut Criterion) {
    use osc_midi_bridge::types::BridgeStatus;

    let status = BridgeStatus {
        running: true,
        midi_input: Some("Test MIDI".to_string()),
        midi_output: Some("Test Output".to_string()),
        osc_target: "127.0.0.1:9000".to_string(),
        rtp_active: true,
        rtp_bound_port: Some(5004),
        rtp_advertised_host: Some("oscmidi-rtp.local.".to_string()),
        rtp_advertised_addresses: vec!["127.0.0.1".to_string()],
        rtp_network_warning: None,
        last_error: None,
        vst_loaded: true,
        audio_running: true,
        audio_latency_ms: Some(5.2),
        audio_backend: Some("ASIO".to_string()),
        audio_device: Some("Test Device".to_string()),
        audio_sample_rate: Some(48_000),
        audio_buffer_size: Some(256),
        audio_requested_buffer_size: Some(256),
        audio_stream_buffer_size: Some(256),
        audio_buffer_mismatch: Some(false),
        vst_midi_compatible: Some(true),
        audio_xruns: Some(0),
        audio_midi_drops: Some(0),
        audio_lock_misses: Some(0),
        audio_emergency_resets: Some(0),
        audio_callback_max_us: Some(0),
        audio_callback_last_us: Some(0),
        audio_callback_over_budget_count: Some(0),
        audio_mmcss_enabled: Some(true),
        audio_power_throttling_disabled: Some(true),
        audio_limiter_enabled: Some(false),
        ..BridgeStatus::default()
    };

    c.bench_function("bridge_status_serialize", |b| {
        b.iter(|| {
            black_box(serde_json::to_string(&status).unwrap());
        })
    });

    let serialized = serde_json::to_string(&status).unwrap();
    c.bench_function("bridge_status_deserialize", |b| {
        b.iter(|| {
            black_box(serde_json::from_str::<BridgeStatus>(&serialized).unwrap());
        })
    });
}

/// Benchmark du traitement MIDI
fn bench_midi_processing(c: &mut Criterion) {
    use osc_midi_bridge::midi::{parse_note, parse_sustain};

    let note_messages: Vec<Vec<u8>> = (0..1000)
        .map(|i| {
            let note = (i % 128) as u8;
            let velocity = if i % 2 == 0 { 100 } else { 0 };
            let status = if velocity > 0 { 0x90 } else { 0x80 };
            vec![status, note, velocity]
        })
        .collect();

    let sustain_messages: Vec<Vec<u8>> = (0..16)
        .map(|channel| vec![0xB0 | channel, 64, 127])
        .collect();

    c.bench_function("parse_note", |b| {
        b.iter(|| {
            for msg in &note_messages {
                black_box(parse_note(msg));
            }
        })
    });

    c.bench_function("parse_sustain", |b| {
        b.iter(|| {
            for msg in &sustain_messages {
                black_box(parse_sustain(msg));
            }
        })
    });
}

criterion_group!(
    benches,
    bench_midi_frame_parsing,
    bench_audio_engine_operations,
    bench_bridge_operations,
    bench_json_serialization,
    bench_midi_processing
);
criterion_main!(benches);
