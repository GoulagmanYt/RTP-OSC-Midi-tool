// The desktop build reports failures in its log/UI and must not spawn a second console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ::tauri::{Emitter, Manager};
use osc_midi_bridge::{
    plugin_probe,
    tauri::{commands::*, state::AppState, window::handle_window_event},
    types,
};

fn main() {
    if let Some(exit_code) = plugin_probe::probe_cli_exit_code() {
        if exit_code == 0 {
            return;
        }
        std::process::exit(exit_code);
    }
    std::panic::set_hook(Box::new(|info| {
        let msg = match info.payload().downcast_ref::<&str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => &s[..],
                None => "Box<Any>",
            },
        };
        let location = info
            .location()
            .map(|l| format!("file '{}' at line {}", l.file(), l.line()))
            .unwrap_or("unknown location".into());
        let err_msg = format!("PANIC: '{}' at {}", msg, location);
        eprintln!("{}", err_msg);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("panic.log")
        {
            use std::io::Write;
            let _ = writeln!(
                file,
                "[{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                err_msg
            );
        }
    }));

    env_logger::init();

    if let Err(error) = run_app() {
        log::error!("Fatal application error: {error}");
        eprintln!("Fatal application error: {error}");
        std::process::exit(1);
    }
}

fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    ::tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .on_window_event(|window, event| {
            #[cfg(target_os = "windows")]
            {
                handle_window_event(event, window);
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = (window, event);
            }
        })
        .invoke_handler(::tauri::generate_handler![
            get_config,
            save_config,
            list_midi_inputs,
            list_midi_outputs,
            start_bridge,
            stop_bridge,
            panic_midi,
            get_status,
            restart_rtp,
            preflight_check,
            refresh_rtp_sessions,
            ping_audio,
            reload_vst,
            reset_config_defaults,
            export_config,
            export_diagnostics,
            import_config,
            get_app_paths,
            open_app_dir,
            clear_log_file,
            list_audio_backends,
            list_audio_devices,
            list_vst_plugins,
            refresh_vst_plugins,
            list_vst_parameters,
            set_vst_parameter,
            open_vst_ui,
            close_vst_ui,
            set_master_gain,
            set_audio_limiter,
            send_test_midi,
            run_automated_stress_test
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "main webview window was not created",
                )
            })?;

            #[cfg(target_os = "windows")]
            {
                window.set_decorations(false)?;
                window.set_always_on_top(false)?;
                window.set_resizable(true)?;
                window.set_title("")?;
            }
            // UI readiness is telemetry only and must not prevent startup if no listener exists yet.
            window
                .emit("log:entry", types::LogEvent::new("info", "Interface ready"))
                .ok();
            Ok(())
        })
        .run(::tauri::generate_context!())
        .map_err(Into::into)
}
