# OSCMidi

Bridge desktop (Tauri + React + Rust) pour router du MIDI entre:
- RTP-MIDI (session reseau)
- peripheriques MIDI locaux
- plugins VST2/VST3 instruments
- sortie OSC

L'application cible principalement Windows (ASIO/WASAPI, VST2/VST3, UI Tauri).

## Fonctionnalites principales

- Bridge RTP-MIDI local avec decouverte/reconnexion.
- Mapping MIDI/OSC (routing, filtres de canaux, profils).
- Moteur audio bas-latence avec backend auto, ASIO ou WASAPI.
- Chargement VST2/VST3 avec UI plugin (quand disponible).
- Scan des plugins et cache local.
- Filtrage strict des plugins instruments:
  - les effets/non compatibles sont ignores et non affiches dans la liste app.
- Sauvegarde de config YAML + etat runtime + logs.
- Export de diagnostics.

## Etat actuel des plugins

- Les plugins VST3 non instruments sont filtres.
- Cas specifique `sforzando.vst3`:
  - workaround actif pour eviter un crash de teardown du plugin lors du `Stop Bridge`.
  - le host n'applique pas de restauration d'etat VST3 interne pour sforzando
    (sforzando garde son propre etat via ses fichiers ARIA, ex `default.ariax`).

## Stack technique

- Frontend: React 19, TypeScript, Vite 8, Tailwind.
- Desktop shell: Tauri v2.
- Backend: Rust.
- Audio/MIDI:
  - `cpal` (ASIO/WASAPI),
  - `midir`,
  - RTP-MIDI (`rtpmidi`, `mdns-sd`),
  - VST2 (`vst`),
  - VST3 via `rack` (vendor patch local dans `vendor/rack`).

## Prerequis (Windows)

- Node.js 20+ (npm inclus)
- Rust toolchain stable (`rustup`, `cargo`)
- Visual Studio Build Tools C++ (MSVC)
- ASIO SDK local (deja reference dans `.cargo/config.toml`)

## Lancement dev

Depuis la racine du repo:

```bash
npm install
npm run tauri:dev
```

## Build release

### Methode recommandee (script Windows)

```bat
build_windows.bat
```

Sortie principale:
- `D:\PROGRAMMATION\oscMIDI\.cargo-target\release\OSCMidi.exe`

Logs build:
- `build_logs\build-*-OK.log` / `build_logs\build-*-FAIL.log`

### Methode manuelle

```bash
npm run build
npm run tauri:build
```

## Architecture du projet

```text
oscMIDI/
  src/                     # Frontend React
  src-tauri/               # Backend Rust + Tauri commands
  vendor/rack/             # Fork patch local pour VST3
  scripts/                 # Scripts utilitaires
  build_windows.bat        # Build complet Windows
```

Points backend importants:
- `src-tauri/src/main.rs`: commandes Tauri exposees au frontend.
- `src-tauri/src/audio.rs`: moteur audio, lifecycle VST, UI plugin.
- `src-tauri/src/plugin_probe.rs`: scan/probe plugins + filtre instrument.
- `vendor/rack/rack-sys/src/vst3_instance.cpp`: lifecycle VST3 natif.

## Donnees utilisateur / chemins

- Config:
  - `%APPDATA%\OSCMIDI\OSCMIDI\config\config.yaml`
- Log app:
  - `%APPDATA%\OSCMIDI\OSCMIDI\config\app.log`
- Etat plugins:
  - `%APPDATA%\OSCMIDI\OSCMIDI\config\vst_state\`

## Troubleshooting

### 1) `ERR_CONNECTION_REFUSED` (localhost)

Cause typique: executable dev lance a la place d'un vrai build release.

Correction:
1. Fermer toutes les instances `OSCMidi.exe`.
2. Rebuild:
   - `npm run tauri:build` ou `build_windows.bat`
3. Lancer uniquement:
   - `.cargo-target\\release\\OSCMidi.exe`

### 2) Un plugin n'apparait pas dans la liste

L'app n'affiche que les plugins instruments supportes.
Un effet VST2/VST3 (ex noise reduction) est volontairement masque.

### 3) Message xruns / dropouts

- Augmenter le buffer (`256` -> `480` -> `512`)
- Verifier le backend (ASIO en priorite)
- Fermer les apps audio concurrentes
- Verifier que le plugin choisi est bien instrument et stable

### 4) sforzando VST3 et banques SFZ

- Verifier les chemins de banques dans la config sforzando/ARIA (`default.ariax`).
- Eviter des references vers `Downloads` si les banques sont installees ailleurs.

## Commandes utiles

```bash
# Typecheck frontend
npm run lint

# Build frontend uniquement
npm run build

# Build app desktop
npm run tauri:build

# Verifier backend Rust
cd src-tauri
cargo check --bin OSCMidi
```

## Contrib

1. Branch depuis `main`
2. Commit clair (scope + action)
3. Build/verification locale avant PR
4. Inclure logs/tests minimaux dans la PR

## Licence

Aucune licence explicite n'est definie dans le repo pour le moment.
