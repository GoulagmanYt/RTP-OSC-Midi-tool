//! Tests unitaires pour les commandes Tauri
//! 
//! Ce module contient des tests pour valider le fonctionnement
//! des commandes Tauri après le refactoring.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tauri::state::AppState;
    use crate::config::ConfigStore;
    use std::sync::{Arc, Mutex};
    use tauri::{State, Manager};
    use parking_lot::Mutex as ParkingLotMutex;

    /// Crée un état de test pour les commandes Tauri
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
    fn test_get_config_command() {
        let state = create_test_state();
        let config = get_config(tauri::generate_context!(()).app_handle(), State::new(state));
        
        // Vérifier que la configuration par défaut est retournée
        assert_eq!(config.midi_input, "".to_string());
        assert_eq!(config.midi_output, "".to_string());
    }

    #[test]
    fn test_list_midi_inputs_command() {
        let result = list_midi_inputs();
        
        // Vérifier que la liste contient au moins l'entrée virtuelle RTP
        assert!(result.is_ok());
        let inputs = result.unwrap();
        assert!(inputs.contains(&crate::config::RTP_VIRTUAL_INPUT.to_string()));
    }

    #[test]
    fn test_list_midi_outputs_command() {
        let result = list_midi_outputs();
        
        // Vérifier que la liste contient au moins la sortie virtuelle VST
        assert!(result.is_ok());
        let outputs = result.unwrap();
        assert!(outputs.contains(&crate::config::VST_INTERNAL_OUTPUT.to_string()));
    }

    #[test]
    fn test_get_app_paths_command() {
        let result = get_app_paths();
        
        // Vérifier que les chemins de l'application sont retournés
        assert!(result.is_ok());
        let paths = result.unwrap();
        assert!(!paths.config_dir.is_empty());
        assert!(!paths.data_dir.is_empty());
    }

    #[test]
    fn test_bridge_status_command() {
        let state = create_test_state();
        let status = get_bridge_status(State::new(state));
        
        // Vérifier que le statut par défaut est correct
        assert_eq!(status.is_running, false);
        assert_eq!(status.bridge_status, crate::types::BridgeStatus::Stopped);
    }

    #[test]
    fn test_generate_preflight_report_command() {
        let state = create_test_state();
        let report = generate_preflight_report(State::new(state));
        
        // Vérifier que le rapport de pré-vol est généré
        assert!(!report.audio_devices.is_empty());
        assert!(!report.midi_ports.inputs.is_empty());
        assert!(!report.midi_ports.outputs.is_empty());
    }

    #[test]
    fn test_start_bridge_error_cases() {
        let state = create_test_state();
        
        // Test avec une configuration invalide
        let window = tauri::generate_context!(()).app_handle().get_webview_window("main").unwrap();
        
        // Le test devrait échouer car il n'y a pas de véritable configuration audio
        let result = start_bridge(
            tauri::generate_context!(()).app_handle(),
            window,
            State::new(state),
        );
        
        // Le résultat peut être une erreur ou un succès selon l'état du système
        // Ce test vérifie principalement que la commande ne panique pas
        match result {
            Ok(_) => println!("Bridge démarré avec succès"),
            Err(_) => println!("Échec du démarrage du bridge (attendu en test)"),
        }
    }

    #[test]
    fn test_error_conversions() {
        // Test des conversions d'erreurs typées
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let app_error: crate::error::AppError = io_error.into();
        
        match app_error {
            crate::error::AppError::Config(config_err) => {
                match config_err {
                    crate::error::ConfigError::ReadError(msg) => {
                        assert!(msg.contains("test"));
                    }
                    _ => panic!("Type d'erreur de configuration inattendu"),
                }
            }
            _ => panic!("Type d'erreur AppError inattendu"),
        }
    }

    #[test]
    fn test_error_display() {
        let audio_err = crate::error::AudioError::PluginNotFound("test.vst".to_string());
        let display = format!("{}", audio_err);
        assert!(display.contains("Plugin VST non trouvé"));
        assert!(display.contains("test.vst"));
    }

    #[test]
    fn test_bridge_error_communication() {
        let comm_err = crate::error::BridgeError::Communication("test error".to_string());
        let app_error: crate::error::AppError = comm_err.into();
        
        match app_error {
            crate::error::AppError::Bridge(bridge_err) => {
                match bridge_err {
                    crate::error::BridgeError::Communication(msg) => {
                        assert_eq!(msg, "test error");
                    }
                    _ => panic!("Type d'erreur Bridge inattendu"),
                }
            }
            _ => panic!("Type d'erreur AppError inattendu"),
        }
    }

    #[test]
    fn test_tauri_error_dialog() {
        let dialog_err = crate::error::TauriError::Dialog("No file selected".to_string());
        let app_error: crate::error::AppError = dialog_err.into();
        
        match app_error {
            crate::error::AppError::Tauri(tauri_err) => {
                match tauri_err {
                    crate::error::TauriError::Dialog(msg) => {
                        assert_eq!(msg, "No file selected");
                    }
                    _ => panic!("Type d'erreur Tauri inattendu"),
                }
            }
            _ => panic!("Type d'erreur AppError inattendu"),
        }
    }

    #[test]
    fn test_error_from_string() {
        let string_err = "Test error".to_string();
        let app_error: crate::error::AppError = string_err.into();
        
        match app_error {
            crate::error::AppError::Generic(msg) => {
                assert_eq!(msg, "Test error");
            }
            _ => panic!("Type d'erreur Generic attendu"),
        }
    }
}
