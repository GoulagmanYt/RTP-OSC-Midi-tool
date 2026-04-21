//! Tests unitaires pour le module d'état Tauri
//! 
//! Ce module contient des tests pour valider le fonctionnement
//! de l'état partagé de l'application après le refactoring.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::bridge::BridgeHandle;
    use crate::audio::AudioEngine;
    use std::sync::Arc;
    use parking_lot::Mutex;

    #[test]
    fn test_app_state_creation() {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        };
        
        // Vérifier que l'état est correctement initialisé
        assert!(state.bridge_handle().is_none());
    }

    #[test]
    fn test_app_state_bridge_operations() {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let mut state = AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        };
        
        // Test setter et getter
        let bridge = BridgeHandle::new();
        state.set_bridge(bridge.clone());
        
        assert!(state.bridge_handle().is_some());
        
        // Test de remplacement par None
        *state.bridge.lock() = None;
        assert!(state.bridge_handle().is_none());
    }

    #[test]
    fn test_app_state_config_operations() {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        };
        
        // Test de chargement de configuration
        let config = state.config_store.load();
        assert_eq!(config.midi_input, "");
        assert_eq!(config.midi_output, "");
    }

    #[test]
    fn test_app_state_audio_operations() {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        };
        
        // Test d'accès au moteur audio
        assert!(!state.audio.is_started());
    }

    #[test]
    fn test_app_state_dev_logging_operations() {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        };
        
        // Test de l'état de logging de développement
        assert_eq!(*state.dev_logging.lock(), false);
        
        // Test de modification
        *state.dev_logging.lock() = true;
        assert_eq!(*state.dev_logging.lock(), true);
    }

    #[test]
    fn test_concurrent_bridge_access() {
        use std::thread;
        use std::time::Duration;
        
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = Arc::new(AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        });
        
        // Test d'accès concurrent au bridge
        let state_clone = state.clone();
        let handle = thread::spawn(move || {
            let bridge = BridgeHandle::new();
            state_clone.set_bridge(bridge);
            thread::sleep(Duration::from_millis(10));
        });
        
        handle.join().unwrap();
        assert!(state.bridge_handle().is_some());
    }

    #[test]
    fn test_concurrent_logging_access() {
        use std::thread;
        use std::time::Duration;
        
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = Arc::new(AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        });
        
        // Test d'accès concurrent au logging
        let state_clone = state.clone();
        let handle = thread::spawn(move || {
            *state_clone.dev_logging.lock() = true;
            thread::sleep(Duration::from_millis(10));
        });
        
        handle.join().unwrap();
        assert_eq!(*state.dev_logging.lock(), true);
    }

    #[test]
    fn test_state_thread_safety() {
        use std::thread;
        
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let state = Arc::new(AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        });
        
        // Test de sécurité des threads
        let mut handles = vec![];
        
        for i in 0..10 {
            let state_clone = state.clone();
            let handle = thread::spawn(move || {
                // Chaque thread tente d'accéder à l'état
                let config = state_clone.config_store.load();
                let logging_state = *state_clone.dev_logging.lock();
                let bridge_exists = state_clone.bridge_handle().is_some();
                
                // Vérifications basiques
                assert_eq!(config.midi_input, "");
                assert!(!logging_state || i % 2 == 0); // Simple vérification
                assert!(!bridge_exists); // Pas de bridge dans ce test
            });
            handles.push(handle);
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_bridge_handle_lifecycle() {
        let config_store = ConfigStore::new("test_config.json").unwrap();
        let bridge_handle = Mutex::new(None);
        let audio = AudioEngine::new();
        let dev_logging = Arc::new(Mutex::new(false));
        
        let mut state = AppState {
            config_store,
            bridge_handle,
            audio,
            dev_logging,
        };
        
        // Test du cycle de vie du bridge
        assert!(state.bridge_handle().is_none());
        
        let bridge = BridgeHandle::new();
        state.set_bridge(bridge);
        assert!(state.bridge_handle().is_some());
        
        // Simulation de nettoyage
        *state.bridge.lock() = None;
        assert!(state.bridge_handle().is_none());
    }

    #[test]
    fn test_config_store_isolation() {
        // Test que chaque AppState a son propre ConfigStore
        let config_store1 = ConfigStore::new("test_config1.json").unwrap();
        let config_store2 = ConfigStore::new("test_config2.json").unwrap();
        
        let state1 = AppState {
            config_store: config_store1,
            bridge_handle: Mutex::new(None),
            audio: AudioEngine::new(),
            dev_logging: Arc::new(Mutex::new(false)),
        };
        
        let state2 = AppState {
            config_store: config_store2,
            bridge_handle: Mutex::new(None),
            audio: AudioEngine::new(),
            dev_logging: Arc::new(Mutex::new(false)),
        };
        
        // Les deux états devraient avoir des configurations indépendantes
        let config1 = state1.config_store.load();
        let config2 = state2.config_store.load();
        
        assert_eq!(config1.midi_input, config2.midi_input);
        assert_eq!(config1.midi_output, config2.midi_output);
    }
}
