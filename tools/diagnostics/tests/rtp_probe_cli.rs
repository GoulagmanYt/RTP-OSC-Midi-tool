use std::process::Command;

#[test]
fn ephemeral_probe_reports_its_bound_control_and_data_ports() {
    let output = Command::new(env!("CARGO_BIN_EXE_rtp_probe"))
        .args(["--port", "0", "--duration", "1", "--idle-timeout", "0"])
        .output()
        .expect("run RTP probe");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let ports = stdout
        .lines()
        .find_map(|line| {
            line.strip_prefix("RTP probe listening:")?
                .split_whitespace()
                .find_map(|field| field.strip_prefix("ports="))
        })
        .expect("probe listening ports");
    let (control, data) = ports.split_once('/').unwrap();
    let control: u16 = control.parse().unwrap();
    let data: u16 = data.parse().unwrap();
    assert_ne!(
        control, 0,
        "probe must report the selected port, not the requested zero"
    );
    assert_eq!(data, control + 1);
}
