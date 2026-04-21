# Architecture OSC-MIDI Bridge

Ce document décrit l'architecture globale du projet OSC-MIDI Bridge après le refactoring complet.

## Vue d'Ensemble

OSC-MIDI Bridge est une application desktop construite avec Tauri qui permet de faire le pont entre les messages MIDI et les messages OSC (Open Sound Control), avec support pour les plugins VST.

### Technologies Principales
- **Frontend** : HTML/CSS/JavaScript avec Tauri
- **Backend** : Rust avec Tauri
- **Audio** : CPAL pour les streams audio, support VST
- **MIDI** : midir pour les ports MIDI
- **OSC** : Support OSC natif
- **RTP** : Communication réseau pour MIDI

## Architecture Modulaire

### Structure des Modules

```
src/
|-- main.rs                 # Point d'entrée de l'application
|-- lib.rs                  # Bibliothèque publique
|-- error.rs                # Système d'erreurs typées
|-- types.rs                # Types partagés
|-- config.rs               # Gestion de configuration
|-- logger.rs               # Système de logging
|-- midi.rs                 # Traitement MIDI
|-- osc.rs                  # Traitement OSC
|-- rtp.rs                  # Communication RTP
|-- plugin_probe.rs         # Détection de plugins
|-- vst_scan.rs             # Scan VST
|-- audio/                  # Sous-module audio
|   |-- mod.rs
|   |-- engine.rs           # Moteur audio principal
|   |-- callback.rs         # Callbacks temps réel
|   |-- config.rs           # Configuration audio
|   |-- plugin.rs           # Interface plugins
|   |-- stream.rs           # Gestion des streams
|   |-- windows.rs          # Spécifique Windows
|-- bridge/                 # Sous-module bridge
|   |-- mod.rs
|   |-- config.rs           # Configuration bridge
|   |-- midi.rs             # Interface MIDI
|   |-- osc.rs              # Interface OSC
|   |-- rtp_bridge.rs       # Bridge RTP
|   |-- activity.rs         # Suivi d'activité
|   |-- runtime.rs          # Runtime du bridge
|-- tauri/                  # Sous-module Tauri
|   |-- mod.rs              # Point d'entrée
|   |-- commands.rs         # Commandes Tauri
|   |-- state.rs            # État partagé
|   |-- utils.rs            # Utilitaires
|   |-- window.rs           # Gestion fenêtres
```

## Flux de Données

### 1. Initialisation
```
main.rs
    -> AppState::new()
    -> Tauri Builder
    -> setup_main_window()
    -> Application démarrée
```

### 2. Configuration
```
ConfigStore
    -> Chargement depuis fichier
    -> Validation
    -> Distribution aux modules
```

### 3. Pipeline Audio
```
Entrée Audio
    -> AudioEngine
    -> Plugin VST
    -> Callback temps réel
    -> Sortie Audio
```

### 4. Pipeline MIDI
```
Entrée MIDI
    -> MidiManager
    -> MidiFrame
    -> BridgeHandle
    -> RTP/OSC
```

### 5. Pipeline RTP/OSC
```
BridgeHandle
    -> RtpManager / OscManager
    -> Réseau
    -> Participants distants
```

## Composants Principaux

### AudioEngine
**Responsabilités** :
- Gestion des streams audio CPAL
- Chargement et gestion des plugins VST
- Callbacks temps réel pour le traitement audio
- Métriques de performance audio

**Caractéristiques** :
- Thread-safe avec Arc<Mutex<>>
- Support pour différents backends audio
- Gestion des erreurs typées
- Optimisation pour faible latence

### BridgeHandle
**Responsabilités** :
- Coordination entre MIDI, OSC et RTP
- Gestion du cycle de vie du bridge
- Suivi de l'activité MIDI
- Synchronisation des participants

**Caractéristiques** :
- Architecture événementielle
- Support multi-protocoles
- État partagé thread-safe
- Métriques en temps réel

### ConfigStore
**Responsabilités** :
- Persistance de la configuration
- Validation des paramètres
- Notification des changements
- Gestion des chemins de fichiers

**Caractéristiques** :
- Sauvegarde atomique
- Validation à la lecture
- Support de la migration
- Gestion des erreurs robuste

### AppState
**Responsabilités** :
- État partagé de l'application
- Coordination entre modules
- Gestion des ressources
- Point d'accès centralisé

**Caractéristiques** :
- Thread-safe avec parking_lot::Mutex
- Cycle de vie géré
- Accès optimisé
- Nettoyage automatique

## Patterns Architecturaux

### 1. Module Pattern
Chaque domaine fonctionnel est organisé en module avec :
- Interface publique claire
- Implémentation privée
- Tests unitaires intégrés
- Documentation complète

### 2. State Management Pattern
Utilisation de `AppState` pour :
- Centraliser l'état partagé
- Éviter les variables globales
- Garantir la thread-safety
- Faciliter les tests

### 3. Error Handling Pattern
Système d'erreurs typées pour :
- Sécurité des types
- Contexte riche
- Performance
- Maintenabilité

### 4. Command Pattern (Tauri)
Commandes Tauri pour :
- Interface frontend/backend
- Validation des entrées
- Gestion asynchrone
- Sécurité

## Performance et Optimisations

### 1. Gestion de la Mémoire
- Utilisation de `Arc` pour le partage sans copie
- `parking_lot::Mutex` pour meilleure performance
- Éviter les allocations inutiles
- Nettoyage explicite des ressources

### 2. Concurrence
- Architecture multi-thread
- Communication via channels
- État partagé protégé
- Callbacks temps réel optimisés

### 3. Latence
- Callbacks audio en temps réel
- Priorités appropriées des threads
- Éviter les allocations dans les callbacks
- Optimisation Windows spécifique

## Sécurité

### 1. Validation des Entrées
- Types forts pour les données
- Validation des paramètres
- Sanitization des chemins
- Gestion des permissions

### 2. Isolation
- Séparation claire des modules
- Interface publique contrôlée
- Pas de variables globales
- Gestion des ressources

### 3. Erreurs
- Pas de paniques non contrôlées
- Gestion gracieuse des erreurs
- Messages d'erreur contextuels
- Logging approprié

## Testing Strategy

### 1. Tests Unitaires
- Chaque module a ses tests
- Couverture des cas limites
- Tests de concurrence
- Tests d'erreurs

### 2. Tests Intégrés
- Tests de bout en bout
- Tests de performance
- Tests de stress
- Tests de compatibilité

### 3. Tests de Régression
- Suite de tests automatisée
- Tests après chaque changement
- Validation des performances
- Tests de compatibilité

## Déploiement et Distribution

### 1. Build Process
- Compilation optimisée
- Minification des assets
- Signature des binaires
- Gestion des dépendances

### 2. Packaging
- Binaires natifs
- Dépendances incluses
- Installation simplifiée
- Mises à jour automatiques

### 3. Support Multi-Plateforme
- Windows (principal)
- Support Linux
- Support macOS (futur)
- Adaptations spécifiques

## Maintenance et Évolution

### 1. Documentation
- Code documenté
- Architecture documentée
- Guides de contribution
- Examples d'utilisation

### 2. Monitoring
- Métriques de performance
- Logs structurés
- Diagnostics intégrés
- Rapports d'erreurs

### 3. Évolution
- Architecture extensible
- Migration progressive
- Rétrocompatibilité
- Versioning sémantique

## Décisions Techniques

### 1. Rust + Tauri
- Performance de Rust
- Interface web moderne
- Distribution simplifiée
- Écosystème riche

### 2. Architecture Modulaire
- Maintenabilité
- Testabilité
- Réutilisabilité
- Collaboration

### 3. Erreurs Typées
- Sécurité
- Performance
- Contexte
- Outils

### 4. Concurrency
- Performance multi-core
- Réactivité
- Scalabilité
- Robustesse

## Références

- [Tauri Documentation](https://tauri.app/)
- [Rust Book](https://doc.rust-lang.org/book/)
- [CPAL Documentation](https://docs.rs/cpal/)
- [midir Documentation](https://docs.rs/midir/)
- [OSC Specification](http://opensoundcontrol.org/spec-1_0/)

---

Cette architecture est conçue pour évoluer avec les besoins de l'application tout en maintenant une haute qualité de code, de performance et de maintenabilité.
