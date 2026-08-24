#![cfg(all(target_os = "windows", feature = "worker-fixtures"))]

use std::time::Duration;

use osc_midi_bridge::{
    audio::AudioSettings,
    types::VstPluginEntry,
    vst_worker::{VstWorkerState, VstWorkerSupervisor},
};

#[test]
fn production_worker_supports_isolated_probe_mode() {
    let probe_path = std::env::temp_dir().join(format!(
        "oscmidi-missing-probe-plugin-{}.dll",
        std::process::id()
    ));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_vst-host-worker"))
        .arg("--vst-probe")
        .arg(&probe_path)
        .output()
        .expect("production worker should start in probe mode");
    assert!(
        output.status.success(),
        "worker probe mode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("probe output should be UTF-8");
    let json = stdout
        .lines()
        .find_map(|line| line.strip_prefix("OSCMIDI_PROBE_JSON:"))
        .expect("worker should emit a probe result");
    let entries: Vec<VstPluginEntry> =
        serde_json::from_str(json).expect("worker should emit a valid probe result");
    let entry = entries
        .first()
        .expect("probe should return one failure entry");
    assert_eq!(entry.path, probe_path.to_string_lossy());
    assert!(!entry.supported);
}

fn fixture_settings() -> AudioSettings {
    AudioSettings {
        enabled: true,
        backend: Some("asio".into()),
        device: Some("fixture".into()),
        device_id: None,
        sample_rate: 48_000,
        buffer_size: 512,
        gain_db: 0.0,
        limiter_enabled: false,
        vst_plugin_id: None,
        vst_path: Some("fixture.vst3".into()),
    }
}

async fn wait_for_exhausted_restart_budget(supervisor: &VstWorkerSupervisor) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut stable_since = None;
    loop {
        let snapshot = supervisor.snapshot();
        let exhausted = snapshot.restarts == 3 && snapshot.state == VstWorkerState::Faulted;
        if exhausted {
            let since = stable_since.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() >= Duration::from_millis(750) {
                return;
            }
        } else {
            stable_since = None;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "worker restart budget did not settle: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn isolated_worker_survives_midi_and_contains_process_crashes() {
    let fixture = env!("CARGO_BIN_EXE_vst-worker-fixture");
    std::env::set_var("OSCMIDI_VST_WORKER_PATH", fixture);
    std::env::set_var("OSCMIDI_VST_WORKER_INHERIT_STDIO", "1");
    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "ready");

    let supervisor = VstWorkerSupervisor::new();
    let status = supervisor
        .start(fixture_settings(), Some("fixture-class".into()))
        .await
        .expect("fixture worker should become ready");
    assert_eq!(status.stream_buffer_size, 512);
    assert_eq!(supervisor.snapshot().state, VstWorkerState::Ready);
    for sequence in 0..10_000u32 {
        let note = 36 + (sequence % 48) as u8;
        assert!(supervisor.try_send_midi(&[0x90, note, 100]));
    }
    supervisor.stop().await.expect("fixture worker stop");
    assert_eq!(supervisor.snapshot().state, VstWorkerState::Disabled);

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "crash");
    let crashed = VstWorkerSupervisor::new();
    assert!(crashed.start(fixture_settings(), None).await.is_err());
    wait_for_exhausted_restart_budget(&crashed).await;
    let snapshot = crashed.snapshot();
    assert_eq!(snapshot.state, VstWorkerState::Faulted);
    assert_eq!(snapshot.restarts, 3);
    assert!(snapshot.last_exit.is_some());

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "hang");
    let hung = VstWorkerSupervisor::new();
    hung.start(fixture_settings(), None)
        .await
        .expect("hang fixture first becomes ready");
    let detection_started = std::time::Instant::now();
    while hung.snapshot().restarts == 0 && detection_started.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        detection_started.elapsed() < Duration::from_millis(2_000),
        "hung worker was not detected within the 1.5 second heartbeat budget"
    );
    wait_for_exhausted_restart_budget(&hung).await;
    assert_eq!(hung.snapshot().restarts, 3);
    hung.stop().await.ok();

    std::env::remove_var("OSCMIDI_VST_FIXTURE_MODE");
    std::env::set_var(
        "OSCMIDI_VST_WORKER_PATH",
        env!("CARGO_BIN_EXE_vst-host-worker"),
    );
    let real_worker = VstWorkerSupervisor::new();
    let invalid_load = real_worker.start(fixture_settings(), None).await;
    assert!(
        invalid_load.is_err(),
        "the real worker must reject a nonexistent fixture VST"
    );
    assert_eq!(real_worker.snapshot().state, VstWorkerState::Faulted);
    real_worker.stop().await.ok();

    std::env::remove_var("OSCMIDI_VST_WORKER_PATH");
    std::env::remove_var("OSCMIDI_VST_WORKER_INHERIT_STDIO");
}
