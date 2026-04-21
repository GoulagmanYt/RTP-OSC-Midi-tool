# Module Tauri - Interface Frontend/Backend

Ce module contient toute la logique spécifique à Tauri qui a été refactorisée depuis `main.rs`. Il est organisé en sous-modules pour une meilleure maintenabilité.

## Structure du Module

```
tauri/
|-- commands.rs      # Commandes Tauri exposées au frontend
|-- commands_tests.rs # Tests unitaires pour les commandes
|-- state.rs         # État partagé de l'application
|-- state_tests.rs   # Tests unitaires pour l'état
|-- utils.rs         # Fonctions utilitaires Tauri
|-- utils_tests.rs   # Tests unitaires pour les utilitaires
|-- window.rs        # Gestion des fenêtres et événements
|-- mod.rs           # Point d'entrée du module
```

## Fonctionnalités

### Commands (`commands.rs`)
Les commandes Tauri sont l'interface entre le frontend JavaScript/HTML et le backend Rust. Elles incluent :

- **Gestion de configuration** : `get_config`, `set_config`, `import_config`, `export_config`
- **Contrôle du bridge** : `start_bridge`, `stop_bridge`, `get_bridge_status`
- **Gestion MIDI** : `list_midi_inputs`, `list_midi_outputs`, `send_midi_frame`
- **Gestion RTP** : `list_rtp_participants`, `test_rtp_connectivity`
- **Utilitaires** : `get_app_paths`, `generate_preflight_report`, `export_diagnostics`

### État (`state.rs`)
L'état partagé de l'application contient :

- **ConfigStore** : Gestion de la configuration persistante
- **BridgeHandle** : Référence au bridge MIDI/RTP
- **AudioEngine** : Moteur audio pour les plugins VST
- **DevLogging** : État du logging de développement

### Utilitaires (`utils.rs`)
Fonctions utilitaires pour :

- **Configuration audio** : Conversion des paramètres de configuration
- **Synchronisation RTP** : Gestion du manager de découverte RTP
- **Diagnostics** : Export d'informations système
- **VST** : Scan des plugins VST disponibles

### Fenêtres (`window.rs`)
Gestion des fenêtres et événements :

- **Configuration initiale** : Setup de la fenêtre principale
- **Événements** : Gestion des événements de fenêtre
- **Décorations Windows** : Personnalisation de l'apparence

## Erreurs Typées

Le module utilise un système d'erreurs typées (`error::AppError`) pour une meilleure gestion des erreurs :

- **AudioError** : Erreurs spécifiques au moteur audio
- **BridgeError** : Erreurs du bridge MIDI/RTP
- **ConfigError** : Erreurs de configuration
- **TauriError** : Erreurs de l'interface Tauri

## Tests

Chaque module dispose de tests unitaires complets :

```bash
# Exécuter tous les tests Tauri
cargo test --lib tauri

# Tests spécifiques
cargo test --lib tauri::commands
cargo test --lib tauri::state
cargo test --lib tauri::utils
```

## Utilisation

### Depuis main.rs
```rust
use crate::tauri::{
    commands::*,
    state::AppState,
    utils::setup_main_window,
    window::handle_window_event,
};

// Configuration du builder Tauri
tauri::Builder::default()
    .manage(AppState::new()?)
    .invoke_handler(generate_handler![
        get_config,
        set_config,
        start_bridge,
        stop_bridge,
        // ... autres commandes
    ])
    .setup(|app| {
        setup_main_window(app.handle())?;
        Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

### Depuis le Frontend
```javascript
// Appel des commandes Tauri depuis JavaScript
import { invoke } from '@tauri-apps/api/tauri';

// Obtenir la configuration
const config = await invoke('get_config');

// Démarrer le bridge
await invoke('start_bridge');

// Envoyer un frame MIDI
await invoke('send_midi_frame', { 
    frame: { 
        source: "test", 
        data: [0x90, 60, 100] 
    } 
});
```

## Performance et Optimisations

### État Partagé
- Utilisation de `parking_lot::Mutex` pour une meilleure performance
- Accès concurrent sécurisé avec `Arc<Mutex<T>>`
- Éviter les copies inutiles avec des références

### Commandes Asynchrones
- Les commandes d'E/S (import/export) sont asynchrones
- Utilisation de callbacks pour les dialogues de fichiers
- Gestion appropriée des erreurs avec les types d'erreurs

### Mémoire
- Partage de l'état avec `Arc` pour éviter les duplications
- Utilisation de `Mutex` uniquement où nécessaire
- Nettoyage approprié des ressources

## Sécurité

### Validation des Entrées
- Les commandes Tauri valident les paramètres d'entrée
- Utilisation de types forts pour éviter les erreurs
- Gestion des erreurs avec des messages appropriés

### Accès Fichiers
- Utilisation des API Tauri sécurisées pour les dialogues
- Validation des chemins de fichiers
- Gestion des permissions appropriée

## Maintenance

### Ajout de Nouvelles Commandes
1. Ajouter la fonction dans `commands.rs`
2. Décorer avec `#[::tauri::command]`
3. Ajouter au `generate_handler!` dans `main.rs`
4. Ajouter des tests dans `commands_tests.rs`

### Modification de l'État
1. Mettre à jour `AppState` dans `state.rs`
2. Adapter les accesseurs si nécessaire
3. Mettre à jour les tests dans `state_tests.rs`
4. Vérifier la compatibilité avec les commandes existantes

### Nouveaux Utilitaires
1. Ajouter la fonction dans `utils.rs`
2. Documenter avec des commentaires `///`
3. Ajouter des tests dans `utils_tests.rs`
4. Exporter si nécessaire depuis `mod.rs`

## Débogage

### Logging
- Utiliser `log::debug!` pour les informations de débogage
- Le logging de développement peut être activé via la configuration
- Les erreurs sont loggées avec contexte approprié

### Diagnostics
- La commande `export_diagnostics` fournit des informations système complètes
- Inclut l'état du bridge, audio, et RTP
- Utile pour le support et le débogage

## Migration depuis main.rs

Ce module remplace la logique Tauri qui était précédemment dans `main.rs` :

- **Avant** : Toute la logique dans un seul fichier
- **Après** : Organisation modulaire avec séparation des responsabilités
- **Avantages** : Meilleure maintenabilité, testabilité, et réutilisabilité

Les changements sont rétrocompatibles au niveau de l'API Tauri exposée au frontend.
