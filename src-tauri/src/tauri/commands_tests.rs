use super::*;

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
    assert!(err.contains("Dossier inconnu"));
}

#[test]
fn wrappers_return_serializable_json() {
    let app_paths = serde_json::to_string(&get_app_paths().expect("paths")).expect("json");
    assert!(app_paths.contains("configDir"));
}
