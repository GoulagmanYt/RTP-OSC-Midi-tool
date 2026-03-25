# OSCMidi

Desktop bridge (Tauri + React + Rust) to route MIDI between:
- RTP-MIDI sessions (network)
- local MIDI devices
- VST2/VST3 instrument plugins
- OSC output

Primary target is Windows (ASIO/WASAPI, VST2/VST3, Tauri desktop app).

## Features

- RTP-MIDI bridge with discovery and reconnection.
- MIDI routing profiles (channel filters, note/CC/program mapping).
- Low-latency audio engine with auto backend selection, ASIO or WASAPI.
- VST2/VST3 instrument hosting, including plugin editor window when available.
- Plugin scanning and local cache.
- Strict instrument filtering (effects/non-compatible plugins are hidden).
- YAML config persistence, runtime state persistence, and diagnostics export.

## Project Stack

- Frontend: React 19, TypeScript, Vite 8, Tailwind.
- Desktop shell: Tauri v2.
- Backend: Rust.
- Audio/MIDI:
  - `cpal` (ASIO/WASAPI)
  - `midir`
  - RTP-MIDI (`rtpmidi`, `mdns-sd`)
  - VST2 (`vst`)
  - VST3 via patched Rack host (`vendor/rack`)

## Requirements (Windows)

- Node.js 20+ (npm included)
- Rust stable toolchain (`rustup`, `cargo`)
- Visual Studio Build Tools C++ (MSVC)
- Local ASIO SDK (already wired in `.cargo/config.toml`)

## Quick Start

From repository root:

```bash
npm install
npm run tauri:dev
```

## Build

### Recommended (full Windows script)

```bat
build_windows.bat
```

Main output executable:
- `D:\PROGRAMMATION\oscMIDI\.cargo-target\release\OSCMidi.exe`

### Manual build

```bash
npm run build
npm run tauri:build
```

## Verification Commands

```bash
# Frontend type-check
npm run lint

# Frontend production build
npm run build

# Rust tests (debug)
cd src-tauri
cargo test

# Rust tests (release)
cargo test --release

# Static analysis
cargo clippy --all-targets --all-features
```

## Project Layout

```text
oscMIDI/
  src/                     # React frontend
  src-tauri/               # Rust backend + Tauri commands
  vendor/rack/             # Local VST3 hosting patch
  scripts/                 # Utility scripts
  build_windows.bat        # Full Windows release build
```

Key backend files:
- `src-tauri/src/main.rs`: Tauri command entry points.
- `src-tauri/src/bridge.rs`: MIDI routing pipeline.
- `src-tauri/src/audio.rs`: audio engine + plugin lifecycle/editor handling.
- `src-tauri/src/plugin_probe.rs`: plugin scanning/probing and compatibility filtering.
- `src-tauri/src/rtp.rs`: RTP-MIDI server and discovery manager.

## Runtime Data Paths

- Config:
  - `%APPDATA%\OSCMIDI\OSCMIDI\config\config.yaml`
- App log:
  - `%APPDATA%\OSCMIDI\OSCMIDI\config\app.log`
- Plugin state:
  - `%APPDATA%\OSCMIDI\OSCMIDI\config\vst_state\`

## Plugin Notes

- Non-instrument VST3 plugins are intentionally filtered out.
- `sforzando.vst3` has a dedicated teardown workaround to avoid plugin unload crashes.
- Internal VST3 state restore is not applied for sforzando; it keeps state through ARIA files (for example `default.ariax`).

## Troubleshooting

### `ERR_CONNECTION_REFUSED` on localhost

Typical cause: launching an old dev binary instead of the latest release build.

Fix:
1. Close all `OSCMidi.exe` instances.
2. Rebuild with `npm run tauri:build` (or `build_windows.bat`).
3. Launch only `.cargo-target\release\OSCMidi.exe`.

### A plugin does not appear in the list

Only supported instrument plugins are shown. Effects are hidden by design.

### XRuns / dropouts

- Increase audio buffer (`256` -> `480` -> `512`).
- Prefer ASIO backend when available.
- Close competing real-time audio applications.
- Test with a known stable instrument plugin.

### sforzando VST3 SFZ banks not loading

- Check ARIA/sforzando paths (`default.ariax`).
- Avoid stale absolute paths (for example old `Downloads` paths).

## Contributing

1. Branch from `main`.
2. Make focused commits with clear messages.
3. Run local verification before opening a PR.
4. Include test/build evidence in the PR description.

## License

No explicit license file is currently present in this repository.
