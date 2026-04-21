//! Interface unifiée pour les plugins VST2 et VST3
//! 
//! Ce module fournit une interface commune pour charger et utiliser
//! des plugins VST2 et VST3, avec gestion de l'état et des paramètres.

use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

use crate::types::VstParameter;

/// Erreurs liées aux plugins VST
#[derive(Debug, Error, Clone)]
#[allow(dead_code)]
pub enum PluginError {
    #[error("Plugin non trouvé: {0}")]
    NotFound(String),
    
    #[error("Erreur de chargement du plugin: {0}")]
    LoadError(String),
    
    #[error("Plugin non initialisé")]
    NotInitialized,
    
    #[error("Opération non supportée par ce plugin")]
    UnsupportedOperation,
    
    #[error("Erreur de sauvegarde de l'état: {0}")]
    SaveError(String),
    
    #[error("Erreur de chargement de l'état: {0}")]
    StateLoadError(String),
}

/// Interface unifiée pour les plugins VST
pub trait PluginBackend: Send + Sync {
    /// Traite les samples audio
    #[allow(dead_code)]
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize);
    
    /// Envoie un message MIDI
    #[allow(dead_code)]
    fn send_midi(&mut self, data: &[u8]);
    
    /// Définit un paramètre
    #[allow(dead_code)]
    fn set_parameter(&mut self, index: usize, value: f32) -> Result<(), PluginError>;
    
    /// Retourne la valeur d'un paramètre
    #[allow(dead_code)]
    fn get_parameter(&self, index: usize) -> f32;
    
    /// Retourne tous les paramètres
    #[allow(dead_code)]
    fn get_parameters(&self) -> Vec<VstParameter>;
    
    /// Retourne le nombre de paramètres
    #[allow(dead_code)]
    fn parameter_count(&self) -> usize;
    
    /// Retourne le nom d'un paramètre
    #[allow(dead_code)]
    fn get_parameter_name(&self, index: usize) -> String;
    
    /// Ouvre l'interface graphique
    #[allow(dead_code)]
    fn open_editor(&mut self, parent: Option<std::ptr::NonNull<()>>) -> Result<(), PluginError>;
    
    /// Ferme l'interface graphique
    #[allow(dead_code)]
    fn close_editor(&mut self) -> Result<(), PluginError>;
    
    /// Vérifie si l'interface est ouverte
    #[allow(dead_code)]
    fn is_editor_open(&self) -> bool;
    
    /// Vérifie si le plugin supporte MIDI
    #[allow(dead_code)]
    fn supports_midi(&self) -> bool;
    
    /// Retourne le nombre d'entrées audio
    #[allow(dead_code)]
    fn input_count(&self) -> usize;
    
    /// Retourne le nombre de sorties audio
    #[allow(dead_code)]
    fn output_count(&self) -> usize;
    
    /// Définit le sample rate
    #[allow(dead_code)]
    fn set_sample_rate(&mut self, sample_rate: f32);
    
    /// Définit la taille du buffer
    #[allow(dead_code)]
    fn set_buffer_size(&mut self, buffer_size: usize);
    
    /// Sauvegarde l'état du plugin
    #[allow(dead_code)]
    fn save_state(&self) -> Result<Vec<u8>, PluginError>;
    
    /// Charge l'état du plugin
    #[allow(dead_code)]
    fn load_state(&mut self, data: &[u8]) -> Result<(), PluginError>;
    
    /// Retourne le nom du plugin
    #[allow(dead_code)]
    fn get_name(&self) -> String;
    
    /// Retourne la version du plugin
    #[allow(dead_code)]
    fn get_version(&self) -> String;
    
    /// Retourne le vendeur du plugin
    #[allow(dead_code)]
    fn get_vendor(&self) -> String;
    
    /// Suspend le traitement (pour économiser CPU)
    #[allow(dead_code)]
    fn suspend(&mut self);
    
    /// Reprend le traitement
    #[allow(dead_code)]
    fn resume(&mut self);
}

/// Plugin VST mock pour les tests
#[derive(Debug)]
#[allow(dead_code)]
pub struct MockPlugin {
    name: String,
    supports_midi: bool,
    parameters: Vec<VstParameter>,
    editor_open: bool,
    sample_rate: f32,
    buffer_size: usize,
}

impl MockPlugin {
    /// Crée un nouveau plugin mock
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            name: "Mock Plugin".to_string(),
            supports_midi: true,
            parameters: vec![
                VstParameter {
                    index: 0,
                    name: "Volume".to_string(),
                    value: 0.5,
                    default: 0.5,
                    min: 0.0,
                    max: 1.0,
                    unit: "".to_string(),
                },
                VstParameter {
                    index: 1,
                    name: "Pan".to_string(),
                    value: 0.5,
                    default: 0.5,
                    min: 0.0,
                    max: 1.0,
                    unit: "".to_string(),
                },
            ],
            editor_open: false,
            sample_rate: 44100.0,
            buffer_size: 512,
        }
    }
    
    /// Crée un plugin mock avec configuration personnalisée
    #[allow(dead_code)]
    pub fn with_config(name: String, supports_midi: bool, param_count: usize) -> Self {
        let mut parameters = Vec::new();
        for i in 0..param_count {
            parameters.push(VstParameter {
                index: i,
                name: format!("Param {}", i),
                value: 0.5,
                default: 0.5,
                min: 0.0,
                max: 1.0,
                unit: "".to_string(),
            });
        }
        
        Self {
            name,
            supports_midi,
            parameters,
            editor_open: false,
            sample_rate: 44100.0,
            buffer_size: 512,
        }
    }
}

impl PluginBackend for MockPlugin {
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) {
        // Copie les entrées vers les sorties (pass-through)
        for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
            let len = input.len().min(frames);
            output[..len].copy_from_slice(&input[..len]);
        }
        
        // Remplir les canaux restants de silence
        for output in outputs.iter_mut().skip(inputs.len()) {
            output[..frames].fill(0.0);
        }
    }
    
    fn send_midi(&mut self, _data: &[u8]) {
        // Mock implementation - ne fait rien
    }
    
    fn set_parameter(&mut self, index: usize, value: f32) -> Result<(), PluginError> {
        if index < self.parameters.len() {
            self.parameters[index].value = value.clamp(0.0, 1.0);
            Ok(())
        } else {
            Err(PluginError::UnsupportedOperation)
        }
    }
    
    fn get_parameter(&self, index: usize) -> f32 {
        self.parameters.get(index).map(|p| p.value).unwrap_or(0.0)
    }
    
    fn get_parameters(&self) -> Vec<VstParameter> {
        self.parameters.clone()
    }
    
    fn parameter_count(&self) -> usize {
        self.parameters.len()
    }
    
    fn get_parameter_name(&self, index: usize) -> String {
        self.parameters.get(index).map(|p| p.name.clone()).unwrap_or_default()
    }
    
    fn open_editor(&mut self, _parent: Option<std::ptr::NonNull<()>>) -> Result<(), PluginError> {
        self.editor_open = true;
        Ok(())
    }
    
    fn close_editor(&mut self) -> Result<(), PluginError> {
        self.editor_open = false;
        Ok(())
    }
    
    fn is_editor_open(&self) -> bool {
        self.editor_open
    }
    
    fn supports_midi(&self) -> bool {
        self.supports_midi
    }
    
    fn input_count(&self) -> usize {
        2 // Stéréo par défaut
    }
    
    fn output_count(&self) -> usize {
        2 // Stéréo par défaut
    }
    
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }
    
    fn set_buffer_size(&mut self, buffer_size: usize) {
        self.buffer_size = buffer_size;
    }
    
    fn save_state(&self) -> Result<Vec<u8>, PluginError> {
        // Sauvegarder les paramètres
        let mut data = Vec::new();
        for param in &self.parameters {
            data.extend_from_slice(&param.value.to_le_bytes());
        }
        Ok(data)
    }
    
    fn load_state(&mut self, data: &[u8]) -> Result<(), PluginError> {
        // Charger les paramètres
        let chunk_size = std::mem::size_of::<f32>();
        for (i, param) in self.parameters.iter_mut().enumerate() {
            let start = i * chunk_size;
            if start + chunk_size <= data.len() {
                let bytes = &data[start..start + chunk_size];
                param.value = f32::from_le_bytes(bytes.try_into().unwrap_or([0; 4]));
            }
        }
        Ok(())
    }
    
    fn get_name(&self) -> String {
        self.name.clone()
    }
    
    fn get_version(&self) -> String {
        "1.0.0".to_string()
    }
    
    fn get_vendor(&self) -> String {
        "Mock Vendor".to_string()
    }
    
    fn suspend(&mut self) {
        // Mock implementation
    }
    
    fn resume(&mut self) {
        // Mock implementation
    }
}

/// Chargeur de plugins VST
#[derive(Debug)]
#[allow(dead_code)]
pub struct PluginLoader {
    sample_rate: f32,
    buffer_size: usize,
}

impl PluginLoader {
    /// Crée un nouveau chargeur de plugins
    #[allow(dead_code)]
    pub fn new(sample_rate: f32, buffer_size: usize) -> Self {
        Self {
            sample_rate,
            buffer_size,
        }
    }
    
    /// Charge un plugin VST2
    #[allow(dead_code)]
    pub fn load_vst2(&self, path: &Path) -> Result<Box<dyn PluginBackend>, PluginError> {
        // TODO: Implémenter le chargement VST2 réel
        // Pour l'instant, retourner un plugin mock
        let mut plugin = MockPlugin::with_config(
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown")
                .to_string(),
            true,
            10,
        );
        plugin.set_sample_rate(self.sample_rate);
        plugin.set_buffer_size(self.buffer_size);
        
        Ok(Box::new(plugin))
    }
    
    /// Charge un plugin VST3
    #[allow(dead_code)]
    pub fn load_vst3(&self, path: &Path) -> Result<Box<dyn PluginBackend>, PluginError> {
        // TODO: Implémenter le chargement VST3 réel
        // Pour l'instant, retourner un plugin mock
        let mut plugin = MockPlugin::with_config(
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown")
                .to_string(),
            true,
            15,
        );
        plugin.set_sample_rate(self.sample_rate);
        plugin.set_buffer_size(self.buffer_size);
        
        Ok(Box::new(plugin))
    }
    
    /// Charge un plugin en détectant automatiquement le type
    #[allow(dead_code)]
    pub fn load_auto(&self, path: &Path) -> Result<Box<dyn PluginBackend>, PluginError> {
        if let Some(extension) = path.extension() {
            match extension.to_str() {
                Some("dll") => {
                    // Essayer VST3 d'abord, puis VST2
                    if let Ok(plugin) = self.load_vst3(path) {
                        Ok(plugin)
                    } else {
                        self.load_vst2(path)
                    }
                }
                Some("vst3") => self.load_vst3(path),
                Some("vst") => self.load_vst2(path),
                _ => Err(PluginError::LoadError(format!(
                    "Extension non reconnue: {:?}",
                    extension
                ))),
            }
        } else {
            Err(PluginError::LoadError("Pas d'extension".to_string()))
        }
    }
}

/// Gestionnaire d'état pour les plugins VST
#[allow(dead_code)]
pub struct PluginStateManager {
    /// Plugin VST actuellement chargé
    #[allow(dead_code)]
    plugin: Arc<Mutex<dyn PluginBackend>>,
    /// Chemin vers le plugin VST
    #[allow(dead_code)]
    plugin_path: PathBuf,
}

impl PluginStateManager {
    /// Crée un nouveau gestionnaire d'état
    #[allow(dead_code)]
    pub fn new(plugin: Arc<Mutex<dyn PluginBackend>>, plugin_path: PathBuf) -> Self {
        Self {
            plugin,
            plugin_path,
        }
    }
    
    /// Sauvegarde l'état du plugin sur disque
    #[allow(dead_code)]
    pub fn save_to_disk(&self) -> Result<(), PluginError> {
        let state = self.plugin.lock().save_state()?;
        let path = self.state_path_for_plugin()?;
        
        // Créer le répertoire parent si nécessaire
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PluginError::SaveError(format!("Impossible de créer le répertoire: {}", e))
            })?;
        }
        
        std::fs::write(&path, state).map_err(|e| {
            PluginError::SaveError(format!("Impossible d'écrire le fichier: {}", e))
        })?;
        
        Ok(())
    }
    
    /// Charge l'état du plugin depuis disque
    #[allow(dead_code)]
    pub fn load_from_disk(&self) -> Result<(), PluginError> {
        let path = self.state_path_for_plugin()?;
        
        if !path.exists() {
            return Ok(()); // Pas d'état sauvegardé, c'est normal
        }
        
        let data = std::fs::read(&path).map_err(|e| {
            PluginError::LoadError(format!("Impossible de lire le fichier: {}", e))
        })?;
        
        self.plugin.lock().load_state(&data)?;
        Ok(())
    }
    
    /// Génère le chemin pour l'état du plugin
    #[allow(dead_code)]
    fn state_path_for_plugin(&self) -> Result<PathBuf, PluginError> {
        let file_name = self.plugin_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PluginError::LoadError("Nom de fichier invalide".to_string()))?;
        
        let base_dir = directories::ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
            .map(|d| d.config_dir().join("vst_state"))
            .ok_or_else(|| PluginError::SaveError("Impossible de trouver le répertoire config".to_string()))?;
        
        Ok(base_dir.join(format!("{}.state", file_name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mock_plugin() {
        let mut plugin = MockPlugin::new();
        
        assert_eq!(plugin.get_name(), "Mock Plugin");
        assert!(plugin.supports_midi());
        assert_eq!(plugin.input_count(), 2);
        assert_eq!(plugin.de_output_count(), 2);
        assert_eq!(plugin.parameter_count(), 2);
        
        // Test des paramètres
        assert_eq!(plugin.get_parameter(0), 0.5);
        plugin.set_parameter(0, 0.8).unwrap();
        assert_eq!(plugin.get_parameter(0), 0.8);
        
        // Test de l'interface graphique
        assert!(!plugin.is_editor_open());
        plugin.open_editor(None).unwrap();
        assert!(plugin.is_editor_open());
        plugin.close_editor().unwrap();
        assert!(!plugin.is_editor_open());
    }
    
    #[test]
    fn test_plugin_loader() {
        let loader = PluginLoader::new(44100.0, 512);
        
        // Test avec un chemin factice (retournera un mock)
        let path = PathBuf::from("test.dll");
        let plugin = loader.load_auto(&path);
        assert!(plugin.is_ok());
        
        let plugin = plugin.unwrap();
        assert_eq!(plugin.get_name(), "test.dll");
    }
    
    #[test]
    fn test_plugin_state_manager() {
        let plugin = Arc::new(Mutex::new(MockPlugin::new()));
        let path = PathBuf::from("test.dll");
        let manager = PluginStateManager::new(plugin, path);
        
        // Test de sauvegarde/chargement
        assert!(manager.save_to_disk().is_ok());
        assert!(manager.load_from_disk().is_ok());
    }
}
