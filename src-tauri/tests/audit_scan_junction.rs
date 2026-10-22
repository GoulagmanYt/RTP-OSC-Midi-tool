//! Characterization evidence for directory cycles; no third-party DLL is loaded.
#![cfg(windows)]
use std::{
    fs,
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn scanner_returns_one_physical_dll_through_a_junction_cycle() {
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
fn two_junction_cycles_finish_within_the_probe_budget() {
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
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "scan subprocess failed: {status}");
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
    assert_eq!(
        osc_midi_bridge::plugin_probe::scan_plugin_candidates(std::path::Path::new(&root)).len(),
        1
    );
}

#[test]
fn scanner_deduplicates_hard_linked_dlls() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("instrument.dll");
    fs::write(&original, []).unwrap();
    fs::hard_link(&original, fixture.path().join("alias.dll")).unwrap();
    assert_eq!(
        osc_midi_bridge::plugin_probe::scan_plugin_candidates(fixture.path()).len(),
        1
    );
}

#[test]
fn scanner_skips_nested_junctions_but_accepts_a_junction_root() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("scan");
    let outside = fixture.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("instrument.dll"), []).unwrap();
    fs::write(outside.join("linked.dll"), []).unwrap();
    let junction = root.join("alias");
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside)
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(output.status.success());
    let nested = osc_midi_bridge::plugin_probe::scan_plugin_candidates(&root);
    let selected_root = osc_midi_bridge::plugin_probe::scan_plugin_candidates(&junction);
    fs::remove_dir(&junction).unwrap();
    assert_eq!(nested, vec![root.join("instrument.dll")]);
    assert_eq!(selected_root, vec![junction.join("linked.dll")]);
}
