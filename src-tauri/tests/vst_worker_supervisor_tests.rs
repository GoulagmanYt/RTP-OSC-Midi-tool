#![cfg(all(target_os = "windows", feature = "worker-fixtures"))]

use std::time::Duration;

use osc_midi_bridge::{
    audio::AudioSettings,
    types::VstPluginEntry,
    vst_worker::{VstWorkerState, VstWorkerSupervisor},
    VstEqSettings,
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
        game_mode: true,
        backend: Some("asio".into()),
        device: Some("fixture".into()),
        device_id: None,
        sample_rate: 48_000,
        render_ahead_blocks: 1,
        dsp_thread_priority: Default::default(),
        buffer_size: 512,
        gain_db: 0.0,
        limiter_enabled: false,
        vst_eq: Default::default(),
        vst_plugin_id: None,
        vst_path: Some("fixture.vst3".into()),
    }
}

async fn wait_for_exhausted_restart_budget(supervisor: &VstWorkerSupervisor) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
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
    for enabled in [true, false] {
        assert!(supervisor.try_set_limiter_enabled(enabled));
        // Commands are consumed FIFO: Parameters follows the limiter's Ack.
        supervisor.list_parameters().await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = supervisor.snapshot();
            if snapshot.metrics.limiter_enabled == Some(enabled)
                && snapshot.status.as_ref().unwrap().limiter_enabled == enabled
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "applied limiter state was not published: {snapshot:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    assert!(supervisor.try_set_eq(VstEqSettings {
        enabled: true,
        ..VstEqSettings::default()
    }));
    supervisor.open_editor().await.unwrap();
    assert!(!supervisor.snapshot().editor_open);
    supervisor.set_game_mode(false).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !supervisor.snapshot().editor_open {
        assert!(
            tokio::time::Instant::now() < deadline,
            "deferred editor opening was not published"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    supervisor.close_editor().await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!supervisor.snapshot().editor_open);
    for sequence in 0..10_000u32 {
        let note = 36 + (sequence % 48) as u8;
        assert!(supervisor.try_send_midi(&[0x90, note, 100]));
    }
    supervisor.stop().await.expect("fixture worker stop");
    assert_eq!(supervisor.snapshot().state, VstWorkerState::Disabled);

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "reject-limiter");
    let rejected_limiter = VstWorkerSupervisor::new();
    rejected_limiter
        .start(fixture_settings(), None)
        .await
        .unwrap();
    assert!(rejected_limiter.try_set_limiter_enabled(true));
    rejected_limiter.list_parameters().await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let snapshot = rejected_limiter.snapshot();
    assert_eq!(snapshot.metrics.limiter_enabled, Some(false));
    assert!(!snapshot.status.unwrap().limiter_enabled);
    rejected_limiter.stop().await.unwrap();

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "recover-controls");
    let recovered = VstWorkerSupervisor::new();
    recovered.start(fixture_settings(), None).await.unwrap();
    let mut expected = fixture_settings();
    expected.gain_db = -60.0;
    expected.limiter_enabled = true;
    expected.vst_eq.enabled = true;
    assert!(recovered.try_set_gain(expected.gain_db));
    assert!(recovered.try_set_limiter_enabled(expected.limiter_enabled));
    assert!(recovered.try_set_eq(expected.vst_eq.clone()));
    std::env::set_var(
        "OSCMIDI_VST_FIXTURE_EXPECT_SETTINGS",
        serde_json::to_string(&expected).unwrap(),
    );
    assert!(recovered.save_state().await.is_err());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while recovered.snapshot().restarts == 0 || !recovered.is_ready() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "recovery did not restore live controls: {:?}",
            recovered.snapshot()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    recovered.stop().await.unwrap();
    std::env::remove_var("OSCMIDI_VST_FIXTURE_EXPECT_SETTINGS");

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "slow-command");
    let interrupted = VstWorkerSupervisor::new();
    interrupted.start(fixture_settings(), None).await.unwrap();
    let save = interrupted.save_state();
    tokio::pin!(save);
    // A FIFO barrier proves SaveState was admitted before Stop.
    tokio::select! {
        biased;
        result = &mut save => panic!("SaveState completed before the barrier: {result:?}"),
        result = interrupted.list_parameters() => { result.unwrap(); },
    }
    tokio::time::timeout(Duration::from_secs(2), interrupted.stop())
        .await
        .expect("Stop blocked behind a delayed response")
        .unwrap();
    assert!(save.await.is_err());
    assert_eq!(interrupted.snapshot().state, VstWorkerState::Disabled);
    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "ready");
    interrupted.start(fixture_settings(), None).await.unwrap();
    interrupted.list_parameters().await.unwrap();
    interrupted.stop().await.unwrap();

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "truncated-response");
    let truncated = VstWorkerSupervisor::new();
    truncated.start(fixture_settings(), None).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), truncated.save_state())
            .await
            .expect("truncated response left a pending command")
            .is_err()
    );
    truncated.stop().await.ok();
    assert!(!truncated.is_ready());

    for mode in ["wrong-token", "wrong-abi"] {
        std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", mode);
        let rejected = VstWorkerSupervisor::new();
        let error = rejected.start(fixture_settings(), None).await.unwrap_err();
        assert!(
            error.contains("authentication/protocol mismatch"),
            "{mode}: {error}"
        );
        assert_eq!(rejected.snapshot().state, VstWorkerState::Faulted);
        rejected.stop().await.ok();
    }

    for mode in ["silence-two-seconds", "slow-command"] {
        std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", mode);
        let healthy = VstWorkerSupervisor::new();
        healthy.start(fixture_settings(), None).await.unwrap();
        if mode == "slow-command" {
            healthy.save_state().await.unwrap();
        } else {
            tokio::time::sleep(Duration::from_millis(1700)).await;
            assert!(healthy.snapshot().heartbeat_delayed);
            assert_eq!(healthy.snapshot().restarts, 0);
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        assert_eq!(healthy.snapshot().restarts, 0);
        assert_eq!(healthy.snapshot().state, VstWorkerState::Ready);
        healthy.stop().await.unwrap();
    }
    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "callback-blocked");
    let blocked = VstWorkerSupervisor::new();
    blocked.start(fixture_settings(), None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(5800)).await;
    assert!(blocked.snapshot().restarts > 0);
    assert!(blocked
        .snapshot()
        .last_exit
        .as_deref()
        .unwrap_or_default()
        .contains("callback stalled"));
    blocked.stop().await.ok();

    std::env::set_var("OSCMIDI_VST_FIXTURE_MODE", "legacy");
    let incompatible = VstWorkerSupervisor::new();
    let error = incompatible
        .start(fixture_settings(), None)
        .await
        .unwrap_err();
    assert!(error.contains("protocol mismatch"));
    incompatible.stop().await.ok();

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
    while hung.snapshot().restarts == 0 && detection_started.elapsed() < Duration::from_secs(7) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        detection_started.elapsed() >= Duration::from_secs(5)
            && detection_started.elapsed() < Duration::from_secs(7),
        "hung worker was not detected within the 5 second heartbeat budget"
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
