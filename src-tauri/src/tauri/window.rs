//! Gestion des fenêtres Tauri

#[cfg(target_os = "windows")]
use tauri::WindowEvent;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE},
};

#[cfg(target_os = "windows")]
use window_vibrancy::apply_acrylic;

/// Configure la fenêtre principale
#[cfg(target_os = "windows")]
pub fn setup_main_window(window: &tauri::Window) -> Result<(), Box<dyn std::error::Error>> {
    let hwnd = window.hwnd()?;

    // Appliquer l'effet de transparence (acrylic)
    apply_acrylic(window, Some((0, 0, 0, 0)))?;

    // Configurer les coins de la fenêtre
    unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd.0),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &2i32 as *const _ as *const std::ffi::c_void, // DWMWCP_DONOTROUND = 2
            std::mem::size_of::<i32>() as u32,
        )?;
    }

    Ok(())
}

/// Gestionnaire d'événements pour la fenêtre
#[cfg(target_os = "windows")]
pub fn handle_window_event(event: &WindowEvent, window: &tauri::Window) {
    match event {
        WindowEvent::Focused(focused) => {
            if *focused {
                let _ = setup_main_window(window);
            } else {
                // When the window loses focus, Windows may downgrade the process
                // scheduling (remove foreground boost, apply EcoQoS on Win11).
                // Re-assert HIGH_PRIORITY_CLASS and disable power throttling
                // to prevent audio callback preemption.
                crate::audio::windows_tuning::reapply_process_priority_on_focus_loss();
            }
        }
        WindowEvent::Resized(_) => {
            let _ = setup_main_window(window);
        }
        _ => {}
    }
}

/// Fonction no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn setup_main_window(_window: &tauri::Window) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Fonction no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn handle_window_event(_event: &tauri::WindowEvent, _window: &tauri::Window) {
    // Pas de gestion spécifique sur les autres plateformes
}
