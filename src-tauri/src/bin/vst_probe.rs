use std::path::PathBuf;

fn main() {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: vst_probe <plugin-path>");
        std::process::exit(2);
    };
    let entry = osc_midi_bridge::plugin_probe::probe_plugin(&path);
    match serde_json::to_string(&entry) {
        Ok(json) => println!("OSCMIDI_PROBE_JSON:{json}"),
        Err(error) => {
            eprintln!("failed to serialize probe result: {error}");
            std::process::exit(1);
        }
    }
}
