use super::*;

#[tokio::test(flavor = "current_thread")]
async fn long_command_yields_to_the_calling_thread() {
    let caller = std::thread::current().id();
    let (release, wait) = std::sync::mpsc::channel();
    let command = blocking_command("test.task-failed", "test", move || {
        // The other future can release this simulated long operation only if
        // polling the command leaves the single async thread available.
        let released = wait.recv_timeout(std::time::Duration::from_secs(2)).is_ok();
        Ok((released, std::thread::current().id()))
    });
    let (result, ()) = tokio::join!(biased; command, async move {
        let _ = release.send(());
    });
    let (released, worker) = result.expect("command result");
    assert!(
        released,
        "blocking command prevented its caller from progressing"
    );
    assert_ne!(caller, worker, "service must run on a blocking thread");
}

#[tokio::test(flavor = "current_thread")]
async fn blocking_command_preserves_service_errors() {
    let error = blocking_command("test.task-failed", "test", || {
        Err::<(), _>(CommandError::new(
            "audio.original",
            "audio",
            "service error",
        ))
    })
    .await
    .expect_err("service error");
    assert_eq!(error.code, "audio.original");
    assert_eq!(error.domain, "audio");
}

#[tokio::test(flavor = "current_thread")]
async fn blocking_command_maps_task_panics_to_command_errors() {
    let error = blocking_command("audio.test-task-failed", "audio", || {
        panic!("simulated blocking task failure");
        #[allow(unreachable_code)]
        Ok(())
    })
    .await
    .expect_err("task failure");
    assert_eq!(error.code, "audio.test-task-failed");
    assert_eq!(error.domain, "audio");
}

#[test]
fn get_app_paths_returns_legacy_shape() {
    let paths = get_app_paths().expect("paths");
    assert!(!paths.config_dir.is_empty());
    assert!(!paths.log_file.is_empty());
    assert!(!paths.log_dir.is_empty());
}

#[test]
fn list_midi_inputs_contains_rtp_virtual_input() {
    let inputs = list_midi_inputs().expect("midi inputs");
    assert!(inputs.contains(&crate::config::RTP_VIRTUAL_INPUT.to_string()));
}

#[test]
fn list_midi_outputs_contains_vst_output() {
    let outputs = list_midi_outputs().expect("midi outputs");
    assert!(outputs.contains(&crate::config::VST_INTERNAL_OUTPUT.to_string()));
}

#[test]
fn open_app_dir_rejects_unknown_target() {
    let err = open_app_dir("unknown".to_string()).expect_err("should fail");
    assert_eq!(err.code, "paths.unknown-target");
    assert_eq!(err.domain, "paths");
}

#[test]
fn wrappers_return_serializable_json() {
    let app_paths = serde_json::to_string(&get_app_paths().expect("paths")).expect("json");
    assert!(app_paths.contains("configDir"));
}

#[test]
fn command_error_is_serializable() {
    let err = open_app_dir("unknown".to_string()).expect_err("should fail");
    let json = serde_json::to_string(&err).expect("json");
    assert!(json.contains("\"code\":\"paths.unknown-target\""));
    assert!(json.contains("\"domain\":\"paths\""));
}
