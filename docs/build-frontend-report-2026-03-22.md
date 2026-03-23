# Frontend And Build Optimization Report

Date: 2026-03-22

## Scope

This report documents the optimization work completed in three passes:

1. Critical runtime and build cleanup
2. Frontend bundle splitting and warning cleanup
3. Asset and i18n payload reduction

The goal was to improve:

- frontend startup weight
- chunk structure
- build hygiene
- Rust local build ergonomics
- warning quality during `cargo build` and `tauri build`

## Initial State

Before the second and third passes, the frontend production build had these notable outputs:

- `react-core`: `214.19 kB`
- `ui-kit`: `83.07 kB`
- `ui-vendor`: `72.39 kB`
- `vendor`: `40.93 kB`
- `tauri`: `20.74 kB`
- image asset `pastille.png`: `210.80 kB`

Observed issues:

- command palette code was part of the startup path
- Radix and `cmdk` dependencies were grouped too broadly
- translations lived inline in `LanguageProvider.tsx`
- the app shipped a `400x400` PNG for a `32x32` avatar slot
- Cargo incremental build emitted hard-link warnings because the workspace lived on `D:` formatted as `exFAT`
- Tauri identifier warning existed before cleanup
- `vst` deprecation warnings existed before cleanup

## Pass 1

Primary changes:

- MIDI bridge made stricter and more honest about disconnected ports
- `verbose` logging wired to runtime logging state
- misleading UI options removed or hidden
- VST scan extended to standard Windows folders
- native build chain fixed for `rack` and MSVC CRT compatibility

Key files:

- [bridge.rs](d:\PROGRAMMATION\oscMIDI\src-tauri\src\bridge.rs)
- [logger.rs](d:\PROGRAMMATION\oscMIDI\src-tauri\src\logger.rs)
- [main.rs](d:\PROGRAMMATION\oscMIDI\src-tauri\src\main.rs)
- [vst_scan.rs](d:\PROGRAMMATION\oscMIDI\src-tauri\src\vst_scan.rs)
- [tauri.conf.json](d:\PROGRAMMATION\oscMIDI\src-tauri\tauri.conf.json)

Outcome:

- `cargo build`, `cargo test`, `npm run build`, and `npm run tauri:build` became green

## Pass 2

Primary changes:

- route-level lazy loading added in [App.tsx](d:\PROGRAMMATION\oscMIDI\src\App.tsx)
- vendor chunking introduced in [vite.config.ts](d:\PROGRAMMATION\oscMIDI\vite.config.ts)
- command palette split out of the shell startup path in [Shell.tsx](d:\PROGRAMMATION\oscMIDI\src\components\layout\Shell.tsx) and [CommandPalette.tsx](d:\PROGRAMMATION\oscMIDI\src\components\layout\CommandPalette.tsx)
- Tauri identifier changed to `com.oscmidi.desktop` in [tauri.conf.json](d:\PROGRAMMATION\oscMIDI\src-tauri\tauri.conf.json)
- `vst` deprecation warnings silenced at module boundary in [audio.rs](d:\PROGRAMMATION\oscMIDI\src-tauri\src\audio.rs) and [vst_scan.rs](d:\PROGRAMMATION\oscMIDI\src-tauri\src\vst_scan.rs)
- Cargo target directory moved to NTFS via [.cargo/config.toml](d:\PROGRAMMATION\oscMIDI\.cargo\config.toml)
- Windows build script aligned with the new Cargo target dir in [build_windows.bat](d:\PROGRAMMATION\oscMIDI\build_windows.bat)

Intermediate build result:

- `react-core`: `7.93 kB`
- `react-dom`: `206.12 kB`
- `ui-primitives`: `16.78 kB`
- `ui-select`: `20.27 kB`
- `ui-command`: `46.87 kB`
- `ui-vendor`: `72.43 kB`
- `vendor`: `40.93 kB`

Outcome:

- the previous Vite large chunk warning disappeared
- Cargo exFAT hard-link warnings disappeared after moving `target-dir` to `C:`
- Tauri identifier warning disappeared

## Pass 3

Primary changes:

- translations extracted from `LanguageProvider.tsx` into:
  - [en.ts](d:\PROGRAMMATION\oscMIDI\src\locales\en.ts)
  - [fr.ts](d:\PROGRAMMATION\oscMIDI\src\locales\fr.ts)
  - [types.ts](d:\PROGRAMMATION\oscMIDI\src\locales\types.ts)
- locales now loaded dynamically by [LanguageProvider.tsx](d:\PROGRAMMATION\oscMIDI\src\providers\LanguageProvider.tsx)
- top bar avatar asset replaced with [pastille-96.jpg](d:\PROGRAMMATION\oscMIDI\src\assets\pastille-96.jpg) in [TopBar.tsx](d:\PROGRAMMATION\oscMIDI\src\components\layout\TopBar.tsx)
- old `src/assets/pastille.png` removed

Asset impact:

- old image: `210,808 bytes`
- new image: `2,135 bytes`

Final frontend build output:

- `react-core`: `7.93 kB`
- `react-dom`: `206.12 kB`
- `index`: `16.78 kB`
- `ui-primitives`: `16.78 kB`
- `ui-select`: `20.27 kB`
- `ui-command`: `46.87 kB`
- `ui-vendor`: `72.43 kB`
- `vendor`: `40.93 kB`
- `en`: `13.16 kB`
- `fr`: `14.69 kB`
- `CommandPalette`: `5.33 kB`

Outcome:

- no large image asset remains in the production output
- translations are no longer part of the main startup bundle
- command palette is loaded on demand instead of at app startup

## Before / After Summary

Startup path improvements:

- route screens lazy-loaded
- command palette lazy-loaded
- translations lazy-loaded
- oversized branding asset removed from startup payload

Build hygiene improvements:

- Tauri identifier warning removed
- `vst` deprecation warnings removed from normal build output
- Cargo hard-link warning removed by moving the target directory to NTFS

What remains intentionally untouched:

- `react-dom` is still the heaviest chunk and is expected for a React desktop app
- `ui-vendor` is still significant because it includes the icon and notification stack
- Vite still reports `PLUGIN_TIMINGS`; this is a tooling performance warning, not a bundle correctness problem

## Validation

Validated successfully during the optimization passes:

- `npm run lint`
- `npm run build`
- `cargo build --quiet`
- `cargo test --quiet`
- `npm run tauri:build`

Current packaged executable:

- [OSCMidi.exe](C:\Users\robin\AppData\Local\oscMIDI\cargo-target\release\OSCMidi.exe)

## Recommended Next Steps

If another optimization pass is needed, the highest-yield targets are:

1. audit `ui-vendor` imports, especially icon usage patterns
2. reduce CSS processing cost reported by Vite plugin timings
3. replace any remaining broad utility imports with narrower entry points where available
