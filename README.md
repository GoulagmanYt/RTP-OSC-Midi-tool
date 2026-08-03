# OSCMidi

OSCMidi is a Windows-only desktop bridge for routing MIDI between RTP-MIDI network sessions, local MIDI devices, VST instruments, and OSC output.

> Platform support: Windows only. macOS and Linux are not supported by this project because the current audio, ASIO, VST hosting, packaging, and runtime assumptions are Windows-specific.

## What It Does

- Hosts a Windows desktop app built with Tauri, React, and Rust.
- Bridges RTP-MIDI sessions to local MIDI devices and OSC endpoints.
- Hosts VST2 and VST3 instrument plugins.
- Provides low-latency audio output with ASIO or WASAPI.
- Scans plugins, filters unsupported plugins, and keeps a local plugin cache.
- Persists routing profiles, runtime state, diagnostics, and YAML configuration.

## Current Release

Latest intended release tag: `v1.1.0`.

For tagged releases, GitHub Actions builds the Windows executable and attaches the generated artifacts to the GitHub Release.

## Downloading A Build

From GitHub:

1. Open the repository Actions tab.
2. Select the `Build Windows App` workflow.
3. Open the latest successful run.
4. Download the `OSCMidi-windows-main` artifact.

For release builds, open the GitHub Releases page and download the assets attached to the `v*` release.

The main generated executable is `OSCMidi.exe`. Installer artifacts, such as `.msi`, are uploaded when Tauri produces them.

## Requirements For Local Development

Use Windows 10 or Windows 11.

Required tools:

- Node.js 20 or newer, including npm.
- Rust stable toolchain through `rustup`.
- Visual Studio Build Tools with the C++ MSVC toolchain.
- CMake available on PATH.
- LLVM/Clang with `clang.exe` and `libclang.dll` available on PATH or in the standard install locations.
- Git.
- The ASIO SDK folder included in this repository.

The repository configures Cargo to use `.cargo-target` for build output and `ASIOSDK` for CPAL ASIO support.

## Quick Start

From the repository root:

```powershell
npm install
npm run tauri:dev
```

## Building Locally

Recommended Windows build script:

If you are building on a fresh Windows machine, install the following prerequisites first:

```powershell
winget install OpenJS.NodeJS.LTS
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passThru --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.Windows11SDK.22000"
winget install Kitware.CMake
winget install LLVM.LLVM
```

Then run:

```bat
build_windows.bat
```

Manual build:

```powershell
npm run build
npm run tauri:build
```

Main local output:

```text
.cargo-target\release\OSCMidi.exe
```

## GitHub Release Builds

The GitHub workflow is configured in `.github/workflows/build-windows.yml`.

It runs on:

- every push to `main`;
- every tag matching `v*`, for example `v1.0.0`;
- manual workflow dispatch from GitHub Actions.

To create a release build:

```powershell
git tag v1.0.0
git push origin v1.0.0
```

The tag workflow uploads the build artifact and attaches it to the GitHub Release.

## Verification Commands

Frontend checks:

```powershell
npm run lint
npm run build
npm run test
```

Rust checks:

```powershell
cd src-tauri
cargo test
cargo test --release
cargo clippy --all-targets --all-features
```

## Project Stack

- UI: React 19, TypeScript, Vite 8, Tailwind.
- Desktop shell: Tauri v2.
- Backend: Rust.
- Audio and MIDI:
  - `cpal` with ASIO or WASAPI.
  - `midir` for local MIDI.
  - `rtpmidi` and `mdns-sd` for network MIDI.
  - `vst` for VST2.
  - patched `vendor/rack` host for VST3.

## Project Layout

```text
oscMIDI/
  src/                     React frontend
  src-tauri/               Rust backend and Tauri commands
  vendor/rack/             Local VST3 hosting patch
  ASIOSDK/                 ASIO SDK used by CPAL builds
  scripts/                 Utility scripts
  build_windows.bat        Full Windows release build
```

Important backend areas:

- `src-tauri/src/main.rs`: Tauri entry point.
- `src-tauri/src/bridge/`: MIDI routing pipeline.
- `src-tauri/src/audio/`: audio engine, plugin lifecycle, and editor handling.
- `src-tauri/src/plugin_probe*.rs`: plugin scanning and compatibility filtering.
- `src-tauri/src/rtp*.rs`: RTP-MIDI server and discovery.

## Runtime Data Paths

OSCMidi stores runtime data under the Windows app data directory:

```text
%APPDATA%\OSCMIDI\OSCMIDI\config\config.yaml
%APPDATA%\OSCMIDI\OSCMIDI\config\app.log
%APPDATA%\OSCMIDI\OSCMIDI\config\vst_state\
```

## Plugin Notes

- Only supported instrument plugins are shown.
- Effects and incompatible plugins are hidden intentionally.
- `sforzando.vst3` has a dedicated teardown workaround to avoid plugin unload crashes.
- Internal VST3 state restore is not applied for sforzando. It keeps state through ARIA files, for example `default.ariax`.

## Troubleshooting

### `ERR_CONNECTION_REFUSED` On Localhost

Typical cause: an old dev binary is running instead of the latest release build.

Fix:

1. Close all `OSCMidi.exe` instances.
2. Rebuild with `npm run tauri:build` or `build_windows.bat`.
3. Launch `.cargo-target\release\OSCMidi.exe`.

### Plugin Missing From The List

Only supported instrument plugins are displayed. If a plugin is an effect, fails compatibility checks, or crashes during probing, it is filtered out.

### XRuns Or Audio Dropouts

- Increase the audio buffer size, for example `256` to `480` or `512`.
- Prefer ASIO when an ASIO driver is available.
- Close other real-time audio applications.
- Test with a known stable instrument plugin.

### sforzando SFZ Banks Do Not Load

- Check ARIA and sforzando paths.
- Avoid stale absolute paths, especially paths from temporary folders or old downloads.

## Contributing

1. Branch from `main`.
2. Keep commits focused.
3. Run relevant verification locally.
4. Include build or test evidence in PR descriptions.

## License

No explicit license file is currently present in this repository.
