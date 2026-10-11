use osc_midi_bridge::{Config, ConfigStore, audio::AudioEngine, bridge::BridgeHandle, rtp::RtpDiscoveryManager, tauri::state::AppState};
use std::{io::{Read, Write}, net::TcpStream, sync::{Arc, atomic::AtomicBool}, time::Duration};
use parking_lot::Mutex;
use serde_json::json;

#[test]
fn audit_osc_only_preflight_does_not_require_unused_midi_ports() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState {
        config_store: ConfigStore::with_path(dir.path().join("config.yaml")),
        config_change: Arc::new(Mutex::new(())), bridge: BridgeHandle::new(),
        dev_logging: Arc::new(AtomicBool::new(false)), audio: AudioEngine::new(),
        vst_cache: Mutex::new(None), rtp_discovery: RtpDiscoveryManager::new(),
    };
    let mut cfg = Config::default();
    cfg.osc.input_enabled = true;
    cfg.midi.input_device = None;
    cfg.midi.output_device = None;
    cfg.midi.thru_enabled = false;
    cfg.audio.enabled = false;
    cfg.rtp.enabled = false;
    let report = osc_midi_bridge::application::services::preflight_check(Some(cfg), &state).unwrap();
    assert!(report.midi_in_ok && report.midi_out_ok, "unused MIDI ports block OSC input: {report:?}");
}

#[test]
fn audit_unsupported_config_version_must_not_be_saved_successfully() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::with_path(dir.path().join("config.yaml"));
    let cfg = Config {
        version: Config::CURRENT_VERSION + 1,
        osc: osc_midi_bridge::config::OscConfig {
            target_port: 9876,
            ..Default::default()
        },
        ..Config::default()
    };
    let raw = serde_yaml::to_string(&cfg).unwrap();
    let imported: Result<Config, _> = Config::parse_yaml(&raw);
    assert!(imported.is_err(), "parse_yaml must reject future versions");
    let saved = store.save(&cfg);
    assert!(saved.is_err(), "save must reject future versions");
}

#[test]
fn audit_config_migration_on_parse_yaml() {
    let mut raw_v2 = serde_yaml::to_value(Config::default()).unwrap();
    raw_v2["version"] = serde_yaml::Value::Number(2.into());
    let yaml_v2 = serde_yaml::to_string(&raw_v2).unwrap();
    let parsed_v2 = Config::parse_yaml(&yaml_v2).expect("v2 config should be migrated");
    assert_eq!(parsed_v2.version, Config::CURRENT_VERSION);

    let mut raw_v3 = serde_yaml::to_value(Config::default()).unwrap();
    raw_v3["version"] = serde_yaml::Value::Number(3.into());
    let yaml_v3 = serde_yaml::to_string(&raw_v3).unwrap();
    let parsed_v3 = Config::parse_yaml(&yaml_v3).expect("v3 config should be migrated");
    assert_eq!(parsed_v3.version, Config::CURRENT_VERSION);
}

#[test]
fn audit_export_config_atomic_and_nested_directory() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("nested").join("sub").join("exported.yaml");
    let state = AppState {
        config_store: ConfigStore::with_path(dir.path().join("config.yaml")),
        config_change: Arc::new(Mutex::new(())), bridge: BridgeHandle::new(),
        dev_logging: Arc::new(AtomicBool::new(false)), audio: AudioEngine::new(),
        vst_cache: Mutex::new(None), rtp_discovery: RtpDiscoveryManager::new(),
    };
    let res = osc_midi_bridge::application::services::export_config(target.to_string_lossy().to_string(), &state);
    assert!(res.is_ok(), "export_config should create directories and write: {res:?}");
    assert!(target.exists(), "exported file must exist");
    let content = std::fs::read_to_string(&target).unwrap();
    let reloaded = Config::parse_yaml(&content);
    assert!(reloaded.is_ok(), "reloaded config from export must be valid");
}

fn send(stream: &mut TcpStream, frame: serde_json::Value) {
    let data = serde_json::to_vec(&frame).unwrap();
    stream.write_all(&(data.len() as u32).to_be_bytes()).unwrap();
    stream.write_all(&data).unwrap();
}
fn recv(stream: &mut TcpStream) -> std::io::Result<serde_json::Value> {
    let mut header = [0;4]; stream.read_exact(&mut header)?;
    let mut bytes = vec![0;u32::from_be_bytes(header) as usize]; stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn audit_partial_next_frame_must_not_suppress_playback_completion() {
    let (tx, rx) = osc_midi_bridge::midi_queue::bounded(256);
    let server = osc_midi_bridge::reliable_playback::ReliablePlaybackServer::start("127.0.0.1:0", tx).unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    send(&mut stream, json!({"type":"prepare","version":1,"sessionId":"audit","song":"audit.mid","total":2,
        "events":[{"seq":0,"dueUs":0,"data":[144,60,100]},{"seq":1,"dueUs":100000,"data":[128,60,0]}]}));
    assert_eq!(recv(&mut stream).unwrap()["type"], "prepared");
    send(&mut stream, json!({"type":"start","sessionId":"audit","startDelayMs":50}));
    // TCP may deliver a following Stop frame's header separately from its body.
    stream.write_all(&[0]).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().data[0], 144);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().data[0], 128);
    let completion = recv(&mut stream);
    drop(stream);
    server.stop();
    assert!(completion.is_ok(), "playback finished, completion blocked on the next frame: {completion:?}");
}

#[test]
fn audit_rtp_port_65534_is_accepted_as_candidate() {
    let candidates = osc_midi_bridge::rtp::candidate_ports(65534);
    assert_eq!(candidates.as_slice(), &[65534]);
}
