// Prevents additional console window on Windows in release, DO NOT REMOVE!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod application;
mod audio;
mod bridge;
mod config;
mod error;
mod logger;
mod midi;
mod osc;
mod plugin_probe;
mod reliable_playback;
mod rtp;
mod tauri;
mod types;
mod vst_scan;

use ::tauri::{Emitter, Manager};

use crate::tauri::{commands::*, state::AppState, window::handle_window_event};

/// Fonction principale de l'application
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--vst-probe") {
        let path = std::env::args_os().nth(2).map(std::path::PathBuf::from);
        let Some(path) = path else {
            std::process::exit(2);
        };
        let entry = crate::plugin_probe::probe_plugin(&path);
        if let Ok(json) = serde_json::to_string(&entry) {
            println!("OSCMIDI_PROBE_JSON:{json}");
            return;
        }
        std::process::exit(1);
    }
    // Configuration du gestionnaire de panique
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

    // Initialisation du logger
    env_logger::init();

    // Construction de l'application Tauri avec les modules refactorisés
    ::tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_os::init())
        .manage(AppState::new().expect("Failed to create app state"))
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
            reset_keys,
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
            start_audio,
            stop_audio,
            open_vst_ui,
            close_vst_ui,
            set_master_gain,
            set_audio_limiter,
            send_test_midi,
            run_automated_stress_test
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();

            // Configuration de la fenêtre principale
            #[cfg(target_os = "windows")]
            {
                window.set_decorations(false)?;
                window.set_always_on_top(false)?;
                window.set_resizable(true)?;
                window.set_title("")?;
                // Note: setup_main_window attend &tauri::Window mais nous avons WebviewWindow
                // Pour l'instant, nous sautons cette configuration
            }
            window
                .emit("log:entry", types::LogEvent::new("info", "Interface ready"))
                .ok();
            Ok(())
        })
        .run(::tauri::generate_context!())
        .expect("error while running tauri application");
}
