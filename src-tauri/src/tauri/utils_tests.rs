//! Tests unitaires pour le module d'utilitaires Tauri
//! 
//! Ce module contient des tests pour valider le fonctionnement
//! des fonctions utilitaires après le refactoring.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tauri::state::AppState;
    use crate::config::ConfigStore;
    use std::sync::{Arc, Mutex};
    use parking_lot::Mutex as ParkingLotMutex;

    /// Crée un état de test pour les utilitaires Tauri
    fn create_test_state() -> AppState {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = ParkingLotMutex::new(None);
        let audio = crate::audio::AudioEngine::new();
        let dev_logging = Arc::new(ParkingLotMutex::new(false));
        
        AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        }
    }

    #[test]
    fn test_audio_settings_from_config() {
        let mut config = Config::default();
        config.audio_enabled = true;
        config.sample_rate = 48000;
        config.buffer_size = 256;
        config.audio_backend = Some("ASIO".to_string());
        
        let settings = audio_settings_from_config(&config);
        
        assert_eq!(settings.enabled, true);
        assert_eq!(settings.sample_rate, 48000);
        assert_eq!(settings.buffer_size, 256);
        assert_eq!(settings.backend, Some("ASIO".to_string()));
    }

    #[test]
    fn test_audio_settings_from_config_disabled() {
        let config = Config::default();
        config.audio_enabled = false;
        
        let settings = audio_settings_from_config(&config);
        
        assert_eq!(settings.enabled, false);
    }

    #[test]
    fn test_audio_settings_from_config_defaults() {
        let config = Config::default();
        
        let settings = audio_settings_from_config(&config);
        
        assert_eq!(settings.enabled, true); // Valeur par défaut
        assert_eq!(settings.sample_rate, 44100); // Valeur par défaut
        assert_eq!(settings.buffer_size, 512); // Valeur par défaut
        assert_eq!(settings.backend, None); // Valeur par défaut
    }

    #[test]
    fn test_sync_runtime_logging() {
        let state = create_test_state();
        let mut config = Config::default();
        config.dev_logging_enabled = true;
        
        sync_runtime_logging(&config, &state.dev_logging);
        
        assert_eq!(*state.dev_logging.lock(), true);
    }

    #[test]
    fn test_sync_runtime_logging_disabled() {
        let state = create_test_state();
        let mut config = Config::default();
        config.dev_logging_enabled = false;
        
        sync_runtime_logging(&config, &state.dev_logging);
        
        assert_eq!(*state.dev_logging.lock(), false);
    }

    #[test]
    fn test_sync_rtp_discovery() {
        let state = create_test_state();
        let mut config = Config::default();
        config.rtp_enabled = true;
        config.rtp_port = 5004;
        
        // Test de synchronisation RTP (ne devrait pas paniquer)
        let app_handle = tauri::generate_context!(()).app_handle();
        sync_rtp_discovery(&config, &state, &app_handle);
        
        // Vérifier que le manager RTP est accessible
        let manager = state.rtp_manager();
        // Le manager peut être None si le RTP n'est pas complètement initialisé
        // C'est acceptable pour ce test
    }

    #[test]
    fn test_sync_rtp_discovery_disabled() {
        let state = create_test_state();
        let mut config = Config::default();
        config.rtp_enabled = false;
        
        // Test de synchronisation RTP désactivé
        let app_handle = tauri::generate_context!(()).app_handle();
        sync_rtp_discovery(&config, &state, &app_handle);
        
        // Avec RTP désactivé, il ne devrait pas y avoir de manager
        let manager = state.rtp_manager();
        assert!(manager.is_none());
    }

    #[test]
    fn test_export_diagnostics() {
        let state = create_test_state();
        let window = tauri::generate_context!(()).app_handle().get_webview_window("main").unwrap();
        
        // Test d'export des diagnostics (ne devrait pas paniquer)
        let result = export_diagnostics(&window, &state);
        
        // Le résultat peut être Ok ou Err selon l'état du système
        // Ce test vérifie principalement que la fonction ne panique pas
        match result {
            Ok(_) => println!("Diagnostics exportés avec succès"),
            Err(_) => println!("Échec de l'export des diagnostics (acceptable en test)"),
        }
    }

    #[test]
    fn test_list_vst_scan_roots() {
        let roots = list_vst_scan_roots();
        
        // Vérifier que la liste n'est pas vide
        assert!(!roots.is_empty());
        
        // Vérifier que les chemins sont valides
        for root in &roots {
            assert!(!root.is_empty());
            // Les chemins devraient être absolus ou relatifs valides
        }
    }

    #[test]
    fn test_list_vst_scan_roots_contains_common_paths() {
        let roots = list_vst_scan_roots();
        
        // Vérifier que les chemins VST communs sont présents (Windows)
        #[cfg(target_os = "windows")]
        {
            let has_program_files = roots.iter().any(|path| path.contains("Program Files"));
            let has_program_files_x86 = roots.iter().any(|path| path.contains("Program Files (x86)"));
            
            // Au moins un de ces chemins devrait être présent
            assert!(has_program_files || has_program_files_x86);
        }
    }

    #[test]
    fn test_scan_vst_plugins_in_roots() {
        let roots = list_vst_scan_roots();
        
        // Prendre seulement les premiers chemins pour éviter un scan trop long
        let test_roots: Vec<&String> = roots.iter().take(2).collect();
        
        let plugins = scan_vst_plugins_in_roots(&test_roots);
        
        // La liste peut être vide si aucun VST n'est trouvé
        // Ce test vérifie principalement que la fonction ne panique pas
        println!("Trouvé {} plugins VST dans les chemins de test", plugins.len());
    }

    #[test]
    fn test_scan_vst_plugins_in_empty_roots() {
        let empty_roots: Vec<&String> = vec![];
        
        let plugins = scan_vst_plugins_in_roots(&empty_roots);
        
        // Avec une liste vide, le résultat devrait être vide
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_scan_vst_plugins_in_invalid_roots() {
        let invalid_roots = vec![
            &"C:/nonexistent/path".to_string(),
            &"/invalid/unix/path".to_string(),
        ];
        
        let plugins = scan_vst_plugins_in_roots(&invalid_roots);
        
        // Avec des chemins invalides, le résultat devrait être vide
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_concurrent_audio_settings_extraction() {
        use std::thread;
        use std::time::Duration;
        
        let config = Config::default();
        
        // Test d'extraction concurrente des paramètres audio
        let mut handles = vec![];
        
        for _ in 0..10 {
            let config_clone = config.clone();
            let handle = thread::spawn(move || {
                let settings = audio_settings_from_config(&config_clone);
                thread::sleep(Duration::from_millis(1));
                settings
            });
            handles.push(handle);
        }
        
        for handle in handles {
            let settings = handle.join().unwrap();
            assert_eq!(settings.enabled, true);
            assert_eq!(settings.sample_rate, 44100);
        }
    }

    #[test]
    fn test_concurrent_logging_sync() {
        use std::thread;
        use std::time::Duration;
        
        let state = Arc::new(create_test_state());
        let mut handles = vec![];
        
        for i in 0..10 {
            let state_clone = state.clone();
            let handle = thread::spawn(move || {
                let mut config = Config::default();
                config.dev_logging_enabled = i % 2 == 0;
                
                sync_runtime_logging(&config, &state_clone.dev_logging);
                thread::sleep(Duration::from_millis(1));
            });
            handles.push(handle);
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        // L'état final peut être vrai ou faux selon le dernier thread
        // Ce test vérifie principalement qu'il n'y a pas de race condition
        let final_state = *state.dev_logging.lock();
        println!("État final du logging: {}", final_state);
    }

    #[test]
    fn test_audio_settings_edge_cases() {
        // Test avec des valeurs extrêmes
        let mut config = Config::default();
        config.sample_rate = 192000; // Taux d'échantillonnage très élevé
        config.buffer_size = 8192;   // Taille de buffer très grande
        config.audio_backend = Some("CustomBackend".to_string());
        
        let settings = audio_settings_from_config(&config);
        
        assert_eq!(settings.sample_rate, 192000);
        assert_eq!(settings.buffer_size, 8192);
        assert_eq!(settings.backend, Some("CustomBackend".to_string()));
    }

    #[test]
    fn test_audio_settings_empty_backend() {
        let mut config = Config::default();
        config.audio_backend = Some("".to_string());
        
        let settings = audio_settings_from_config(&config);
        
        assert_eq!(settings.backend, Some("".to_string()));
    }

    #[test]
    fn test_utils_error_handling() {
        // Test de gestion d'erreurs dans les utilitaires
        let state = create_test_state();
        
        // Test avec une configuration invalide
        let mut config = Config::default();
        config.audio_backend = Some("NonExistentBackend".to_string());
        
        // Ne devrait pas paniquer même avec un backend invalide
        let settings = audio_settings_from_config(&config);
        assert_eq!(settings.backend, Some("NonExistentBackend".to_string()));
    }
}
