// Prevents additional console window on Windows in release, DO NOT REMOVE!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod bridge;
mod config;
mod error;
mod logger;
mod midi;
mod osc;
mod plugin_probe;
mod rtp;
mod tauri;
mod types;
mod vst_scan;

use serde::Serialize;
use ::tauri::{Emitter, Manager};

use crate::tauri::{
    commands::*,
    state::AppState,
    utils::config_dir_path,
    window::handle_window_event,
};

/// Chemins de l'application
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub config_dir: String,
    pub log_file: String,
    pub log_dir: String,
}

impl AppPaths {
    /// Crée les chemins de l'application
    pub fn new() -> Result<Self, String> {
        let config_dir = config_dir_path()?;
        let log_dir = config_dir.join("logs");
        let log_file = log_dir.join("app.log");
        
        Ok(Self {
            config_dir: config_dir.to_string_lossy().to_string(),
            log_file: log_file.to_string_lossy().to_string(),
            log_dir: log_dir.to_string_lossy().to_string(),
        })
    }
} // Ajout de l'accolade fermante manquante


/// Fonction principale de l'application
fn main() {
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
            // Commandes du module tauri::commands
            get_config,
            save_config,
            list_midi_inputs,
            list_midi_outputs,
            start_bridge,
            stop_bridge,
            get_bridge_status,
            get_bridge_metrics,
            send_midi_frame,
            list_rtp_participants,
            export_app_diagnostics,
            import_config,
            export_config,
            list_vst_scan_roots,
            scan_vst_plugins,
            load_vst_cache,
            open_folder,
            generate_preflight_report,
            // Commandes locales
            get_app_paths
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
                .emit("log", types::LogEvent::new("info", "Interface ready"))
                .ok();
            Ok(())
        })
        .run(::tauri::generate_context!())
        .expect("error while running tauri application");
}
