# Module Tauri

Le module `tauri/` contient la frontière backend/frontend réellement utilisée par l'application.

## Structure

```text
tauri/
|-- commands.rs       # Contrat Tauri consommé par src/api.ts
|-- commands_tests.rs # Tests unitaires ciblés sur ce contrat
|-- state.rs          # Etat partagé de l'application
|-- utils.rs          # Helpers de chemins, VST cache et synchro runtime
|-- window.rs         # Intégration fenêtre spécifique Tauri/Windows
|-- mod.rs
```

## Commandes publiques

Les commandes exposées au runtime Tauri sont celles consommées par `src/api.ts` :

- `get_config`, `save_config`, `reset_config_defaults`, `import_config`, `export_config`
- `list_midi_inputs`, `list_midi_outputs`
- `start_bridge`, `stop_bridge`, `get_status`, `reset_keys`, `panic_midi`, `send_test_midi`
- `refresh_rtp_sessions`, `restart_rtp`, `preflight_check`
- `list_audio_backends`, `list_audio_devices`
- `list_vst_plugins`, `refresh_vst_plugins`, `list_vst_parameters`, `set_vst_parameter`
- `start_audio`, `stop_audio`, `open_vst_ui`, `close_vst_ui`, `ping_audio`, `reload_vst`
- `set_master_gain`, `set_audio_limiter`
- `get_app_paths`, `open_app_dir`, `clear_log_file`, `export_diagnostics`

Les anciennes commandes ajoutées pendant le refactor intermédiaire ont été retirées du runtime principal pour éviter une surface publique ambiguë.

## Etat partagé

`AppState` agrège :

- `ConfigStore` pour la configuration persistante
- `BridgeHandle` pour le bridge MIDI/OSC/RTP
- `AudioEngine` pour l'audio et le VST
- le cache VST
- `RtpDiscoveryManager`
- l'état du logging de développement

## Vérification

Les validations minimales à conserver pour ce module sont :

```bash
cargo test --test integration_tests test_runtime_no_longer_exports_refactor_only_commands
cargo test --lib tauri::commands
```
