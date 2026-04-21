# Rapport de Complexité - Baseline Phase 1

**Date:** 2026-04-21  
**Projet:** OSC-MIDI Bridge  
**Objectif:** Établir les métriques de base avant refactor

## Métriques Globales

### Taille du Codebase
- **Total fichiers Rust:** 14
- **Total lignes:** 8,145
- **Lignes de code:** 7,351
- **Commentaires:** 97
- **Lignes vides:** 697

### Top 10 Fichiers par Taille
1. **audio.rs** - 2,956 lignes (36% du codebase)
2. **bridge.rs** - 1,119 lignes (14% du codebase)
3. **main.rs** - 795 lignes (10% du codebase)
4. **plugin_probe.rs** - 751 lignes (9% du codebase)
5. **rtp.rs** - 612 lignes (8% du codebase)
6. **config.rs** - 256 lignes (3% du codebase)
7. **bin/rtp_probe.rs** - 224 lignes
8. **logger.rs** - 168 lignes
9. **bin/vst_smoke.rs** - 161 lignes
10. **midi.rs** - 136 lignes

## Problèmes Identifiés

### 1. Fichiers Surdimensionnés
- **audio.rs**: 2,956 lignes - **CRITIQUE** (objectif < 500)
- **bridge.rs**: 1,119 lignes - **CRITIQUE** (objectif < 500)
- **main.rs**: 795 lignes - **ÉLEVÉ** (objectif < 500)

### 2. Fonctions Complexes (identifiées par grep)
Fonctions avec > 7 paramètres identifiées:
- `build_stream()` dans audio.rs (18 paramètres) - **CRITIQUE**
- `build_output_stream_for_sample()` dans audio.rs (18 paramètres) - **CRITIQUE**
- `processing_loop()` dans bridge.rs (11 paramètres) - **ÉLEVÉ**
- `audio_callback()` dans audio.rs (8 paramètres) - **MOYEN**
- `handle_midi_frame()` dans bridge.rs (8 paramètres) - **MOYEN**

### 3. Complexité Cyclomatique Estimée
Basé sur la taille des fichiers et les patterns observés:
- **audio.rs**: Complexité très élevée (fonctions monolithiques)
- **bridge.rs**: Complexité élevée (logique distribuée)
- **main.rs**: Complexité moyenne (logique Tauri mélangée)

## Métriques de Performance (Benchmarks)

### Performance Baseline
```
midi_frame_creation     70.5 µs  (1000 frames)
midi_frame_clone        35.8 µs  (1000 clones)
audio_engine_list_backends  175 ns
audio_engine_list_devices   434 ms  (opération coûteuse)
audio_engine_metrics    42.5 ns
bridge_status_check     289 ns
config_load_save        1.23 ms
bridge_status_serialize 628 ns
bridge_status_deserialize 928 ns
parse_note              3.03 µs (1000 messages)
parse_sustain           23.7 ns
```

### Points de Performance
- **Liste devices audio**: 434ms - **CRITIQUE** (impact UX)
- **Config load/save**: 1.23ms - **MOYEN** (acceptable)
- **Parsing MIDI**: 3µs/1000 messages - **BON**

## Tests Actuels

### Couverture de Tests
- **Tests unitaires existants**: midi.rs, osc.rs
- **Tests d'intégration**: 7 tests créés (Phase 1)
- **Benchmarks**: 5 benchmarks créés (Phase 1)

### Qualité des Tests
- **Tests MIDI**: Excellents (stress tests, parsing)
- **Tests OSC**: Bons (loopback tests)
- **Tests intégration**: Basiques mais couvrent les cycles de vie

## Objectifs de Refactor

### Métriques Cibles
- **Taille max fichier**: < 500 lignes (vs 2,956 actuel)
- **Arguments max fonction**: < 7 (vs 18 actuel)
- **Complexité cyclomatique**: < 10 par fonction
- **Couverture tests**: > 80%

### Actions Prioritaires Phase 2
1. **Extraire audio.rs** en 7 modules séparés
2. **Créer pattern Builder** pour build_stream()
3. **Réduire complexité** de processing_loop()
4. **Séparer logique Tauri** de main.rs

## Risques Identifiés

### Haut Risque
- **audio.rs**: 36% du codebase dans un fichier - refactor complexe
- **Performance**: Liste devices audio (434ms) - ne pas dégrader

### Moyen Risque
- **bridge.rs**: Logique distribuée - risque de régression
- **Tests**: Couverture limitée - besoin plus de tests unitaires

### Faible Risque
- **Fichiers < 500 lignes**: Modifications simples
- **Configuration**: Logique bien isolée

## Prochaine Étape

**Phase 2 Commencée**: Configuration unifiée et pattern Builder

1. Créer `AudioStreamConfig` struct
2. Implémenter `StreamBuilder`
3. Valider que les tests passent
4. Mesurer l'impact sur les benchmarks

---

*Ce rapport servira de baseline pour mesurer l'impact du refactor sur la maintenabilité et la performance.*
