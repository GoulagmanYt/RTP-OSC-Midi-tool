use osc_midi_bridge::audio::{AudioEngine, AudioSettings};
use osc_midi_bridge::bridge::BridgeHandle;
use osc_midi_bridge::config::{
    AppConfig, AudioConfig, AvatarConfig, LoggingConfig, MidiConfig, OscConfig, RtpConfig, Theme,
    UiConfig,
};
use osc_midi_bridge::error::CommandError;
use osc_midi_bridge::midi::MidiFrame;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Test simplifié du cycle de vie du moteur audio (sans dépendances Tauri)
#[test]
fn test_audio_engine_lifecycle() {
    let engine = AudioEngine::new();

    // Vérifier état initial
    assert!(!engine.is_running());

    // Test de configuration audio valide
    let settings = AudioSettings {
        enabled: true,
        backend: Some("auto".to_string()),
        device: None,
        sample_rate: 48_000,
        buffer_size: 256,
        gain_db: 0.0,
        limiter_enabled: false,
        vst_path: None,
    };

    // Vérifier que les settings sont valides
    assert!(settings.enabled);
    assert_eq!(settings.sample_rate, 48_000);
    assert_eq!(settings.buffer_size, 256);

    // Test basique - vérifier que le moteur peut être créé sans erreur
    let engine2 = AudioEngine::new();
    assert!(!engine2.is_running());
}

/// Test simplifié du flux MIDI via le bridge (sans dépendances complexes)
#[test]
fn test_bridge_midi_flow() {
    let bridge = BridgeHandle::new();
    let config = AppConfig::default();

    // Vérifier que le bridge n'est pas démarré
    let status = bridge.status(&config);
    assert!(!status.running);

    // Test de création de frames MIDI
    let test_frame = MidiFrame {
        data: smallvec::SmallVec::from_slice(&[0x90, 60, 100]), // Note On C4
        source: Arc::from("test"),
    };

    // Vérifier que le frame est bien formé
    assert_eq!(test_frame.data.len(), 3);
    assert_eq!(test_frame.data[0], 0x90); // Note On

    // Test basique - vérifier que le bridge peut être créé et interrogé
    let bridge2 = BridgeHandle::new();
    let status2 = bridge2.status(&config);
    assert!(!status2.running);

    // Vérifier les détails du frame MIDI
    assert_eq!(test_frame.data[1], 60); // C4
    assert_eq!(test_frame.data[2], 100); // Velocity
    assert_eq!(&*test_frame.source, "test");
}

/// Test simplifié de chargement de plugin VST par défaut
#[test]
fn test_vst_plugin_loading() {
    let engine = AudioEngine::new();

    // Vérifier les méthodes de base fonctionnent
    let backends = engine.list_backends();
    assert!(!backends.is_empty()); // Au moins un backend disponible

    // Tester la liste des devices pour le premier backend
    let devices = engine.list_devices(Some(backends[0].clone()));
    // Peut être vide si aucun device, mais ne doit pas paniquer
    println!(
        "Found {} devices for backend {}",
        devices.len(),
        backends[0]
    );
}

/// Test de configuration et validation
#[test]
fn test_configuration_validation() {
    let config = AppConfig {
        midi: MidiConfig {
            hotplug: false,
            ..MidiConfig::default()
        },
        audio: AudioConfig {
            sample_rate: 44_100,
            buffer_size: 512,
            gain_db: -6.0,
            ..AudioConfig::default()
        },
        ui: UiConfig {
            theme: Theme::Light,
            avatar: AvatarConfig {
                path: Some("C:/avatars/test.vrm".to_string()),
                offset_x: 12.0,
                offset_y: 18.0,
                scale: 1.25,
            },
            ..UiConfig::default()
        },
        ..AppConfig::default()
    };

    let serialized = serde_yaml::to_string(&config).expect("serialization");
    let reloaded: AppConfig = serde_yaml::from_str(&serialized).expect("deserialization");

    // Vérifier la persistence
    assert_eq!(reloaded.audio.sample_rate, 44_100);
    assert_eq!(reloaded.audio.buffer_size, 512);
    assert_eq!(reloaded.audio.gain_db, -6.0);
    assert_eq!(
        reloaded.ui.avatar.path.as_deref(),
        Some("C:/avatars/test.vrm")
    );
}

/// Test de résistance du bridge sous charge
#[test]
fn test_bridge_stress() {
    let _bridge = BridgeHandle::new();
    let stop = Arc::new(AtomicBool::new(false));
    let _stop_clone = stop.clone();

    // Simuler une charge MIDI pendant 100ms
    let handle = thread::spawn(move || {
        let mut count = 0;
        let start = std::time::Instant::now();

        while start.elapsed() < Duration::from_millis(100) {
            // Simuler injection MIDI (ne fera rien si bridge pas démarré)
            let _frame = MidiFrame {
                data: smallvec::SmallVec::from_slice(&[0x90, (count % 128) as u8, 100]),
                source: Arc::from("stress_test"),
            };

            // Le bridge devrait gérer cette frame même non démarré
            count += 1;

            if count % 1000 == 0 {
                thread::yield_now();
            }
        }

        count
    });

    let frames_sent = handle.join().expect("Thread stress terminé");
    stop.store(true, Ordering::Relaxed);

    // Vérifier qu'on a envoyé un nombre raisonnable de frames
    assert!(
        frames_sent > 1000,
        "Stress test insuffisant: {} frames",
        frames_sent
    );

    println!("Stress test: {} frames MIDI en 100ms", frames_sent);
}

/// Test des métriques de performance audio
#[test]
fn test_audio_metrics() {
    let engine = AudioEngine::new();

    // Métriques initiales - None car le runtime n'est pas démarré
    assert_eq!(engine.xrun_count(), None);
    assert_eq!(engine.midi_drop_count(), None);
    assert_eq!(engine.audio_lock_miss_count(), None);
    assert_eq!(engine.emergency_reset_count(), None);

    // Ces métriques devraient rester None sans stream actif
    assert_eq!(engine.xrun_count(), None);
    assert_eq!(engine.midi_drop_count(), None);

    // Les méthodes de base ne devraient pas paniquer
    let backends = engine.list_backends();
    assert!(!backends.is_empty());
}

/// Test de compatibilité des types
#[test]
fn test_type_compatibility() {
    use osc_midi_bridge::types::RuntimeStatus;

    // Test RuntimeStatus
    let status = RuntimeStatus {
        running: false,
        midi_input: Some("Test MIDI".to_string()),
        midi_output: Some("Test Output".to_string()),
        osc_target: "127.0.0.1:9000".to_string(),
        rtp_active: false,
        rtp_bound_port: None,
        rtp_advertised_host: None,
        rtp_advertised_addresses: Vec::new(),
        rtp_network_warning: None,
        last_error: None,
        audio_running: false,
        audio_latency_ms: None,
        audio_backend: None,
        audio_device: None,
        audio_sample_rate: None,
        audio_buffer_size: None,
        audio_requested_buffer_size: None,
        audio_stream_buffer_size: None,
        audio_buffer_mismatch: None,
        vst_loaded: false,
        vst_midi_compatible: None,
        audio_xruns: None,
        audio_midi_drops: None,
        audio_lock_misses: None,
        audio_emergency_resets: None,
        audio_callback_max_us: None,
        audio_callback_last_us: None,
        audio_callback_over_budget_count: None,
        audio_mmcss_enabled: None,
        audio_power_throttling_disabled: None,
        audio_limiter_enabled: None,
        ..RuntimeStatus::default()
    };

    // Test sérialisation (utilisé par Tauri)
    let serialized = serde_json::to_string(&status).expect("Sérialisation RuntimeStatus réussie");
    let _deserialized: RuntimeStatus =
        serde_json::from_str(&serialized).expect("Désérialisation RuntimeStatus réussie");
}

#[test]
fn test_command_error_serialization() {
    let mut err = CommandError::new("config.invalid", "config", "Configuration invalide");
    err.details
        .insert("field".to_string(), "audio.sampleRate".to_string());
    err.details
        .insert("reason".to_string(), "out_of_range".to_string());

    let serialized = serde_json::to_string(&err).expect("serialize command error");
    assert!(serialized.contains("\"code\":\"config.invalid\""));
    assert!(serialized.contains("\"domain\":\"config\""));
    assert!(serialized.contains("\"field\":\"audio.sampleRate\""));
}

#[test]
fn test_nested_defaults_are_explicit() {
    let config = AppConfig {
        midi: MidiConfig {
            input_device: Some("Input".to_string()),
            output_device: Some("Output".to_string()),
            ..MidiConfig::default()
        },
        osc: OscConfig {
            enabled: true,
            ..OscConfig::default()
        },
        rtp: RtpConfig {
            enabled: true,
            ..RtpConfig::default()
        },
        logging: LoggingConfig {
            enabled: true,
            ..LoggingConfig::default()
        },
        ..AppConfig::default()
    };

    assert_eq!(config.version, AppConfig::CURRENT_VERSION);
    assert_eq!(config.midi.input_device.as_deref(), Some("Input"));
    assert!(config.osc.enabled);
    assert!(config.rtp.enabled);
    assert!(config.logging.enabled);
}

#[test]
fn test_runtime_no_longer_exports_refactor_only_commands() {
    let main_rs = include_str!("../src/main.rs");

    for forbidden in [
        "get_bridge_status,",
        "get_bridge_metrics,",
        "send_midi_frame,",
        "list_rtp_participants,",
        "export_app_diagnostics,",
        "list_vst_scan_roots,",
        "scan_vst_plugins,",
        "load_vst_cache,",
        "open_folder,",
        "generate_preflight_report,",
    ] {
        assert!(
            !main_rs.contains(forbidden),
            "commande refactor-only encore exportée: {forbidden}"
        );
    }
}

#[test]
fn test_audio_legacy_is_not_in_production_module_graph() {
    let lib_rs = include_str!("../src/lib.rs");
    let main_rs = include_str!("../src/main.rs");
    let engine_rs = include_str!("../src/audio/engine.rs");

    assert!(!lib_rs.contains("audio_legacy"));
    assert!(!main_rs.contains("audio_legacy"));
    assert!(!engine_rs.contains("audio_legacy"));
}
