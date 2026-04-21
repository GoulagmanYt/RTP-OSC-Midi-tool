# Module d'Erreurs Typées

Ce module fournit un système d'erreurs typées unifié pour remplacer l'utilisation de `String` comme type d'erreur générique dans toute l'application.

## Objectifs

- **Sécurité** : Types d'erreurs spécifiques au lieu de chaînes génériques
- **Maintenabilité** : Gestion centralisée des erreurs
- **Performance** : Éviter les allocations de chaînes inutiles
- **Débogage** : Messages d'erreur structurés et contextuels

## Structure

```
error/
|-- lib.rs           # Point d'entrée et types principaux
|-- README.md        # Documentation (ce fichier)
```

## Types d'Erreurs

### AppError
Type d'erreur principal qui agrège tous les types d'erreurs spécifiques :

```rust
pub enum AppError {
    Audio(AudioError),
    Bridge(BridgeError),
    Config(ConfigError),
    Tauri(TauriError),
    Generic(String),
}
```

### AudioError
Erreurs spécifiques au moteur audio et aux plugins VST :

```rust
pub enum AudioError {
    PluginNotFound(String),
    PluginNotLoaded,
    StreamError(String),
    InvalidConfig(String),
    CallbackError(String),
    WindowError(String),
    DeviceError(String),
}
```

### BridgeError
Erreurs du bridge MIDI/RTP et de la communication :

```rust
pub enum BridgeError {
    Midi(String),
    Rtp(String),
    Osc(String),
    NotStarted,
    AlreadyRunning,
    Communication(String),
    Processing(String),
}
```

### ConfigError
Erreurs de configuration et de gestion de fichiers :

```rust
pub enum ConfigError {
    FileNotFound,
    ReadError(String),
    WriteError(String),
    ParseError(String),
    Invalid(String),
    InvalidPath(String),
}
```

### TauriError
Erreurs spécifiques à l'interface Tauri :

```rust
pub enum TauriError {
    Command(String),
    State(String),
    Window(String),
    Dialog(String),
    Event(String),
}
```

## Utilisation

### Dans les Commandes Tauri
```rust
use crate::error::{AppError, ConfigError};

#[::tauri::command]
pub async fn import_config() -> Result<Config, AppError> {
    let content = std::fs::read_to_string("config.json")
        .map_err(|e| ConfigError::ReadError(e.to_string()))?;
    
    let config: Config = serde_json::from_str(&content)
        .map_err(|e| ConfigError::ParseError(e.to_string()))?;
    
    Ok(config)
}
```

### Dans le Moteur Audio
```rust
use crate::error::{AppError, AudioError};

pub fn load_plugin(&mut self, path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AudioError::PluginNotFound(
            path.to_string_lossy().to_string()
        ).into());
    }
    
    // ... chargement du plugin
    Ok(())
}
```

### Dans le Bridge
```rust
use crate::error::{AppError, BridgeError};

pub fn start(&mut self) -> Result<(), AppError> {
    if self.is_running() {
        return Err(BridgeError::AlreadyRunning.into());
    }
    
    // ... démarrage du bridge
    Ok(())
}
```

## Conversions

### Conversions Automatiques
Le module fournit des conversions automatiques depuis les types d'erreurs standards :

```rust
// Depuis String
let error: AppError = "Erreur générique".into();

// Depuis std::io::Error
let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "fichier");
let app_error: AppError = io_error.into();

// Depuis serde_json::Error
let json_error = serde_json::from_str::<Config>("invalid").unwrap_err();
let app_error: AppError = json_error.into();
```

### Conversions Manuelles
Pour les conversions plus complexes :

```rust
impl From<crate::audio::engine::AudioError> for AppError {
    fn from(err: crate::audio::engine::AudioError) -> Self {
        AppError::Audio(AudioError::from(err))
    }
}
```

## Macros

### Macro `err!`
Pour créer rapidement des erreurs typées :

```rust
use crate::err;

// Créer une erreur audio
let error = err!(Audio, "Plugin non trouvé");

// Créer une erreur bridge sans message
let error = err!(Bridge);
```

## Performance

### Éviter les Allocations
- Utiliser les variantes sans `String` quand possible
- Préférer `&str` pour les messages constants
- Utiliser `Cow<str>` pour les messages conditionnels

### Exemple Optimisé
```rust
// Moins optimal
pub enum AudioError {
    StreamError(String), // Alloue une String
}

// Plus optimal
pub enum AudioError {
    StreamError(&'static str), // Pas d'allocation
}
```

## Tests

Le module inclut des tests complets pour valider :

- Les conversions entre types d'erreurs
- L'affichage des messages d'erreur
- Les implémentations `Debug` et `Clone`

```bash
# Exécuter les tests du module error
cargo test --lib error
```

## Migration depuis String

### Avant
```rust
pub fn start_bridge() -> Result<(), String> {
    if self.is_running() {
        return Err("Bridge déjà en cours d'exécution".to_string());
    }
    Ok(())
}
```

### Après
```rust
pub fn start_bridge() -> Result<(), AppError> {
    if self.is_running() {
        return Err(BridgeError::AlreadyRunning.into());
    }
    Ok(())
}
```

## Avantages

### 1. Sécurité des Types
- Le compilateur vérifie que tous les cas d'erreur sont gérés
- Pas de messages d'erreur "magiques" à comparer
- Réfactoring plus sûr avec l'aide du compilateur

### 2. Contexte Riche
- Chaque type d'erreur porte son propre contexte
- Messages structurés et cohérents
- Facile à étendre avec de nouvelles variantes

### 3. Performance
- Moins d'allocations de chaînes
- Comparaisons d'erreurs plus rapides
- Taille binaire réduite

### 4. Maintenance
- Centralisation de la logique d'erreur
- Facile à documenter et à tester
- Consistance à travers toute l'application

## Bonnes Pratiques

### 1. Utiliser les Types Spécifiques
Préférer les types d'erreurs spécifiques aux `String` génériques :

```rust
// Bien
Err(ConfigError::FileNotFound.into())

// Éviter
Err("Fichier de configuration non trouvé".to_string().into())
```

### 2. Messages Contextuels
Inclure le contexte pertinent dans les messages :

```rust
// Bien
Err(AudioError::PluginNotFound(format!("Chemin: {}", path.display())).into())

// Moins bien
Err(AudioError::PluginNotFound("plugin".to_string()).into())
```

### 3. Gestion Complète
S'assurer que tous les cas d'erreur sont gérés :

```rust
match app_error {
    AppError::Audio(audio_err) => handle_audio_error(audio_err),
    AppError::Bridge(bridge_err) => handle_bridge_error(bridge_err),
    AppError::Config(config_err) => handle_config_error(config_err),
    AppError::Tauri(tauri_err) => handle_tauri_error(tauri_err),
    AppError::Generic(msg) => handle_generic_error(msg),
}
```

### 4. Tests d'Erreur
Tester les chemins d'erreur explicitement :

```rust
#[test]
fn test_plugin_not_found_error() {
    let result = load_plugin(Path::new("nonexistent.vst3"));
    assert!(matches!(result, Err(AppError::Audio(AudioError::PluginNotFound(_)))));
}
```

## Évolution Future

### Extensions Possibles
- Ajout de nouveaux types d'erreurs spécifiques
- Intégration avec `anyhow` pour plus de flexibilité
- Support pour les codes d'erreur numériques
- Internationalisation des messages d'erreur

### Rétrocompatibilité
- Les conversions depuis `String` sont maintenues
- Migration progressive possible
- Support pour l'ancien API pendant la transition
