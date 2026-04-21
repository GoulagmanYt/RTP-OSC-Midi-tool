# Architecture OSC-MIDI Bridge

Ce document décrit l'architecture réellement compilée après nettoyage du refactor.

## Vue d'ensemble

L'application est un bridge desktop Tauri entre :

- MIDI local
- RTP-MIDI réseau
- OSC vers VRChat
- audio/VST côté backend Rust

Le frontend TypeScript parle uniquement au backend via les commandes définies dans `src/api.ts`.

## Structure backend

```text
src/
|-- main.rs            # Bootstrap Tauri
|-- lib.rs             # Exports publics pour tests et benches
|-- config.rs          # Configuration persistante + normalisation legacy
|-- error.rs           # Types d'erreurs applicatifs
|-- logger.rs          # Logging backend/frontend
|-- midi.rs            # Types et helpers MIDI
|-- osc.rs             # Client OSC
|-- rtp.rs             # Serveur RTP-MIDI + discovery
|-- plugin_probe.rs    # Détection et filtrage de plugins
|-- vst_scan.rs        # Scan des chemins VST
|-- types.rs           # Types sérialisés partagés
|-- audio/
|   |-- mod.rs
|   |-- engine.rs      # Moteur audio/VST de production
|-- bridge/
|   |-- mod.rs
|   |-- runtime.rs     # Runtime bridge de production
|-- tauri/
|   |-- commands.rs
|   |-- state.rs
|   |-- utils.rs
|   |-- window.rs
```

## Principes retenus

- Le contrat frontend est figé par `src/api.ts`.
- Le comportement observable de la sauvegarde historique reste la référence fonctionnelle.
- Le code mort et les façades non branchées ont été retirés du graphe compilé.
- `audio_legacy.rs` n'est plus dans le chemin de production.

## Flux principaux

### Configuration

`ConfigStore` charge et sauvegarde `config.yaml`, puis normalise les anciens champs RTP vers le format courant.

### Audio

`AudioEngine` gère :

- sélection backend/device CPAL
- chargement VST2/VST3
- runtime audio
- métriques audio exposées au bridge et aux commandes Tauri

### Bridge

`BridgeHandle` orchestre :

- entrée MIDI locale
- reset/panic/test MIDI
- routage vers audio, MIDI thru et OSC
- synchronisation RTP-MIDI et participants
- émission des événements d'activité et métriques

### Tauri

`tauri/commands.rs` est la seule frontière de commandes publiques. Les commandes intermédiaires ajoutées pendant le refactor mais non consommées par le frontend ont été retirées.

## Vérification attendue

Les commandes suivantes doivent rester vertes :

```bash
npm run lint
npm run build
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
