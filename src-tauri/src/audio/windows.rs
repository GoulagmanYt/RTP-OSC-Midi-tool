//! Fonctions spécifiques à Windows pour l'audio temps réel
//! 
//! Ce module contient le code Windows-only pour la gestion des priorités
//! temps réel et l'interface graphique des plugins VST.

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(target_os = "windows")]
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::ptr::NonNull;
#[cfg(target_os = "windows")]
use windows::core::{w, PCWSTR};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use crate::logger::background_log;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::HBRUSH;
#[cfg(target_os = "windows")]
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, GetCurrentProcess, GetCurrentThread, ProcessPowerThrottling,
    SetPriorityClass, SetProcessInformation, SetThreadInformation, SetThreadPriority,
    ThreadPowerThrottling, ABOVE_NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, RegisterClassW,
    SetForegroundWindow, ShowWindow, SW_SHOWNORMAL, WNDCLASSW, WS_CAPTION, WS_OVERLAPPEDWINDOW,
    WS_SYSMENU,
};

/// Erreurs spécifiques à Windows
#[derive(Debug, Clone)]
pub enum WindowsError {
    /// Erreur lors de la définition des priorités temps réel
    PriorityError(String),
    
    /// Erreur lors de la création de la fenêtre VST
    WindowError(String),
    
    /// Erreur lors de la récupération du HWND
    HwndError(String),
}

/// Applique les priorités temps réel pour le thread audio
#[cfg(target_os = "windows")]
pub fn apply_realtime_priority() -> Result<(), WindowsError> {
    // Version simplifiée - uniquement la priorité de base
    unsafe {
        let process = GetCurrentProcess();
        SetPriorityClass(process, ABOVE_NORMAL_PRIORITY_CLASS)
            .map_err(|e| WindowsError::PriorityError(format!("SetPriorityClass failed: {}", e)))?;
        
        // Définir les caractéristiques multimédia du thread
        let thread = GetCurrentThread();
        let mut task_index = 0u32;
        let task_name = w!("Pro Audio");
        
        AvSetMmThreadCharacteristicsW(PCWSTR(task_name.as_ptr()), &mut task_index)
            .map_err(|e| WindowsError::PriorityError(format!("AvSetMmThreadCharacteristicsW failed: {}", e)))?;
    }
    
    background_log("info", "Priorité temps réel appliquée au thread audio".to_string());
    Ok(())
}

/// Version no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn apply_realtime_priority() -> Result<(), WindowsError> {
    Ok(())
}

/// Crée une fenêtre pour l'interface VST
#[cfg(target_os = "windows")]
pub fn create_vst_window(
    title: &str,
    width: i32,
    height: i32,
) -> Result<HWND, WindowsError> {
    unsafe {
        // Nom de classe unique pour éviter les conflits
        let class_name = format!("OSCMIDI_VST_{}", std::process::id());
        let class_name_wide: Vec<u16> = class_name.encode_utf16().chain(std::iter::once(0)).collect();
        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        
        // Obtenir l'instance du module
        let hinstance = GetModuleHandleW(None)
            .map_err(|e| WindowsError::WindowError(format!("GetModuleHandleW failed: {}", e)))?;
        
        // Enregistrer la classe de fenêtre
        let wc = WNDCLASSW {
            style: windows::Win32::UI::WindowsAndMessaging::CS_OWNDC,
            lpfnWndProc: Some(def_window_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name_wide.as_ptr()),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
        };
        
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            return Err(WindowsError::WindowError("RegisterClassW failed".to_string()));
        }
        
        // Créer la fenêtre
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(class_name_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CAPTION | WS_SYSMENU,
            0, 0, width, height,
            None, None, Some(hinstance.into()), None,
        )
        .map_err(|e| WindowsError::WindowError(format!("CreateWindowExW failed: {}", e)))?;
        
        // Afficher la fenêtre
        ShowWindow(hwnd, SW_SHOWNORMAL);
        
        Ok(hwnd)
    }
}

/// Version no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn create_vst_window(
    _title: &str,
    _width: i32,
    _height: i32,
) -> Result<std::ptr::NonNull<()>, WindowsError> {
    Err(WindowsError::WindowError("Non supporté sur cette plateforme".to_string()))
}

/// Obtient le HWND de la fenêtre principale de l'application
#[cfg(target_os = "windows")]
pub fn get_main_window_hwnd() -> Result<HWND, WindowsError> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            Err(WindowsError::HwndError("Aucune fenêtre au premier plan".to_string()))
        } else {
            Ok(hwnd)
        }
    }
}

/// Version no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn get_main_window_hwnd() -> Result<std::ptr::NonNull<()>, WindowsError> {
    Err(WindowsError::HwndError("Non supporté sur cette plateforme".to_string()))
}

/// Convertit HWND en NonNull<void> pour compatibilité
#[cfg(target_os = "windows")]
pub fn hwnd_to_nonnull(hwnd: HWND) -> Option<NonNull<()>> {
    if hwnd.is_invalid() {
        None
    } else {
        NonNull::new(hwnd.0 as *mut _)
    }
}

/// Version no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn hwnd_to_nonnull(_hwnd: std::ptr::NonNull<()>) -> Option<NonNull<()>> {
    None
}

/// Ferme proprement une fenêtre VST
#[cfg(target_os = "windows")]
pub fn close_vst_window(hwnd: HWND) -> Result<(), WindowsError> {
    unsafe {
        DestroyWindow(hwnd)
            .map_err(|e| WindowsError::WindowError(format!("DestroyWindow failed: {}", e)))?;
    }
    Ok(())
}

/// Version no-op pour les autres plateformes
#[cfg(not(target_os = "windows"))]
pub fn close_vst_window(_hwnd: std::ptr::NonNull<()>) -> Result<(), WindowsError> {
    Ok(())
}

/// Procédure de fenêtre par défaut
#[cfg(target_os = "windows")]
unsafe extern "system" fn def_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Vérifie si un chemin correspond à Sforzando VST3 (cas spécial)
pub fn is_sforzando_vst3(path: &std::path::Path) -> bool {
    if let Some(name) = path.file_name() {
        if let Some(name_str) = name.to_str() {
            name_str.to_lowercase().contains("sforzando") && path.extension().map_or(false, |ext| ext == "vst3")
        } else {
            false
        }
    } else {
        false
    }
}

/// Timeout pour l'arrêt des plugins VST
pub fn vst_shutdown_timeout() -> std::time::Duration {
    // Permet la configuration via variable d'environnement
    let secs = std::env::var("VST_SHUTDOWN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    
    std::time::Duration::from_secs(secs.max(1).min(30))
}

/// Gestionnaire de fenêtres VST
pub struct VstWindowManager {
    // Stocker le hwnd comme usize pour éviter les problèmes Send/Sync
    hwnd: Option<usize>,
    title: String,
    width: i32,
    height: i32,
}

impl VstWindowManager {
    /// Crée un nouveau gestionnaire de fenêtre
    pub fn new(title: String, width: i32, height: i32) -> Self {
        Self {
            hwnd: None,
            title,
            width,
            height,
        }
    }
    
    /// Crée et affiche la fenêtre
    pub fn create(&mut self) -> Result<(), WindowsError> {
        #[cfg(target_os = "windows")]
        {
            let hwnd = create_vst_window(&self.title, self.width, self.height)?;
            self.hwnd = Some(hwnd.0 as usize);
        }
        
        #[cfg(not(target_os = "windows"))]
        {
            let _ = create_vst_window(&self.title, self.width, self.height)?;
        }
        
        Ok(())
    }
    
    /// Ferme la fenêtre
    pub fn close(&mut self) -> Result<(), WindowsError> {
        if let Some(hwnd_val) = self.hwnd {
            #[cfg(target_os = "windows")]
            {
                let hwnd_ptr = HWND(hwnd_val as *mut std::ffi::c_void);
                close_vst_window(hwnd_ptr)?;
            }
            
            self.hwnd = None;
        }
        Ok(())
    }
    
    /// Retourne le HWND
    pub fn hwnd(&self) -> Option<usize> {
        self.hwnd
    }
    
    /// Met la fenêtre au premier plan
    pub fn bring_to_front(&self) -> Result<(), WindowsError> {
        if let Some(hwnd_val) = self.hwnd {
            #[cfg(target_os = "windows")]
            {
                let hwnd_ptr = HWND(hwnd_val as *mut std::ffi::c_void);
                unsafe {
                    SetForegroundWindow(hwnd_ptr);
                }
            }
        }
        Ok(())
    }
    
    /// Vérifie si la fenêtre est ouverte
    pub fn is_open(&self) -> bool {
        self.hwnd.is_some()
    }
}

impl Drop for VstWindowManager {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sforzando_detection() {
        assert!(is_sforzando_vst3(std::path::Path::new("Sforzando.vst3")));
        assert!(is_sforzando_vst3(std::path::Path::new("sforzando.vst3")));
        assert!(is_sforzando_vst3(std::path::Path::new("SFORZANDO.VST3")));
        assert!(!is_sforzando_vst3(std::path::Path::new("Other.vst3")));
        assert!(!is_sforzando_vst3(std::path::Path::new("Sforzando.dll")));
    }
    
    #[test]
    fn test_vst_shutdown_timeout() {
        let timeout = vst_shutdown_timeout();
        assert!(timeout >= std::time::Duration::from_secs(1));
        assert!(timeout <= std::time::Duration::from_secs(30));
    }
    
    #[test]
    fn test_vst_window_manager() {
        let mut manager = VstWindowManager::new("Test".to_string(), 400, 300);
        assert!(!manager.is_open());
        assert!(manager.hwnd().is_none());
        
        // Note: La création de fenêtre réelle nécessite un environnement graphique
        // Les tests unitaires ne peuvent pas tester cela de manière fiable
    }
}
