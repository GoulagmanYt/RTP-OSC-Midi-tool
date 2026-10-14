//! Characterization evidence for directory cycles; no third-party DLL is loaded.
#![cfg(windows)]
use std::{
    fs,
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn evidence_scanner_revisits_the_same_directory_through_a_junction_cycle() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    fs::write(root.join("instrument.dll"), []).unwrap();
    let ordinary = osc_midi_bridge::plugin_probe::scan_plugin_candidates(&root);
    assert_eq!(ordinary.len(), 1);
    let junction = root.join("loop");
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&root)
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let repeated = osc_midi_bridge::plugin_probe::scan_plugin_candidates(&root);
    // remove_dir removes the junction itself, without traversing its target.
    fs::remove_dir(&junction).unwrap();
    eprintln!(
        "one physical DLL: {} scan candidates through the cycle",
        repeated.len()
    );
    assert_eq!(repeated.len(), 1);
}

#[test]
fn evidence_two_junction_cycles_do_not_finish_within_the_probe_budget() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    fs::write(root.join("instrument.dll"), []).unwrap();
    for name in ["a", "b"] {
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(root.join(name))
            .arg(&root)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let start = Instant::now();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "audit_cycle_scan_child", "--ignored"])
        .env("OSCMIDI_AUDIT_CYCLE_ROOT", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut completed = false;
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            completed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !completed {
        child.kill().unwrap();
    }
    child.wait().unwrap();
    for name in ["a", "b"] {
        fs::remove_dir(root.join(name)).unwrap();
    }
    assert!(
        completed,
        "cyclic scan must complete within the probe budget"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "cyclic scan took {:?}",
        start.elapsed()
    );
}

#[test]
#[ignore = "subprocess helper; requires an isolated junction fixture"]
fn audit_cycle_scan_child() {
    let root = std::env::var_os("OSCMIDI_AUDIT_CYCLE_ROOT").expect("parent fixture");
    osc_midi_bridge::plugin_probe::scan_plugin_candidates(std::path::Path::new(&root));
}
