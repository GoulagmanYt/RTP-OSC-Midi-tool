//! Characterization evidence for docs/AUDIT_BUGS.md, not desired-behavior tests.
//! Replace these assertions with regression expectations when fixing each bug.
use osc_midi_bridge::{
    application::services, audio::AudioEngine, bridge::BridgeHandle, midi::MidiFrame,
    rtp::RtpDiscoveryManager, tauri::state::AppState, Config, ConfigStore,
};
use parking_lot::Mutex;
use std::sync::{atomic::AtomicBool, Arc};

fn state(path: std::path::PathBuf) -> AppState {
    AppState {
        config_store: ConfigStore::with_path(path),
        config_change: Arc::new(Mutex::new(())),
        bridge: BridgeHandle::new(),
        dev_logging: Arc::new(AtomicBool::new(false)),
        audio: AudioEngine::new(),
        vst_cache: Mutex::new(None),
        rtp_discovery: RtpDiscoveryManager::new(),
    }
}

#[test]
fn evidence_panic_without_audio_never_requests_bridge_reset() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path().join("config.yaml"));
    let before = MidiFrame::current_generation();
    services::panic_midi(&state).expect("panic_midi succeeds even without audio running");
    let after = MidiFrame::current_generation();
    assert!(after > before);
}

#[test]
fn evidence_disabled_midi_thru_still_requires_disconnected_output() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path().join("config.yaml"));
    let mut cfg = Config::default();
    cfg.audio.enabled = false;
    cfg.rtp.enabled = false;
    cfg.osc.input_enabled = true;
    cfg.midi.thru_enabled = false;
    cfg.midi.input_device = None;
    cfg.midi.output_device = Some("audit-nonexistent-output-41f654c0".into());
    let report = services::preflight_check(Some(cfg.clone()), &state).unwrap();
    assert!(report.midi_in_ok);
    assert!(report.midi_out_ok);
    cfg.midi.output_device = None;
    assert!(
        services::preflight_check(Some(cfg), &state)
            .unwrap()
            .midi_out_ok
    );
}

#[test]
fn evidence_disabled_osc_target_is_persisted_but_startup_constructor_rejects_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::with_path(dir.path().join("config.yaml"));
    let mut cfg = Config::default();
    cfg.osc.enabled = false;
    cfg.osc.target_ip = String::new();
    store.save(&cfg).unwrap();
    let loaded = store.load();
    assert!(!loaded.osc.enabled);
    assert!(
        osc_midi_bridge::osc::OscClient::new(&loaded.osc.target_ip, loaded.osc.target_port)
            .is_err()
    );
    assert!(osc_midi_bridge::osc::OscClient::for_config(
        loaded.osc.enabled,
        &loaded.osc.target_ip,
        loaded.osc.target_port
    )
    .is_ok());
}
