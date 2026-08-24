# Changelog

All notable changes to OSCMidi are documented here.

## Unreleased

### Changed

- Split pull-request validation into parallel frontend, dependency-audit and Windows Rust jobs while reserving MSI packaging for release-producing events.
- Made pull requests restore the shared Rust cache without creating isolated multi-gigabyte cache entries.
- Consolidated the pending React, Radix UI, Vite, PostCSS, CPAL, Criterion, Chrono and Serde JSON maintenance updates.

### Fixed

- Prevented pull-request artifact uploads from failing when GitHub exposes merge refs containing `/`.
- Updated the locked transitive Nano ID dependency to remove its high-severity zero-length generator advisory.

## 2.5.0 - 2026-08-24

### Changed

- Made successful `main` builds create a missing version tag automatically and publish Windows packages, with Authenticode enabled when the repository signing secrets are configured.
- Made same-version maintenance builds replace the existing release assets while recording their exact commit in `BUILDINFO.json`.
- Extended Windows CI to build and test the application, diagnostics, RTP-MIDI, Rack VST3 library and every VST2 crate target.

### Fixed

- Repaired the vendored VST2 example test and made its plug-in destruction test race-free and compatible with current Rust diagnostics.

## 2.1.0 - 2026-08-10

### Changed

- Made isolated VST hosting resilient to incomplete local VST3 SDK checkouts during clean Windows builds.
- Added release-time PE validation so console-subsystem application or worker binaries cannot be exported.

### Fixed

- Allowed `vst-host-worker.exe` to run the isolated `--vst-probe` subprocess mode instead of misinterpreting probe arguments as a worker IPC session and requiring `--control-pipe`.
- Prevented OSCMidi, its VST probe, the worker, and Explorer helpers from allocating transient Windows consoles in every build profile.
- Created the worker's hidden Tauri host window as invisible from the outset instead of hiding it after its first frame.
- Hid console windows started by third-party VST helper processes while leaving the plug-in editor and unrelated applications visible.
- Prevented VST state restoration during audio startup from being reported as real-time lock incidents.
- Made native-tool warnings non-fatal in Windows PowerShell and generated release checksums without optional PowerShell modules.

## 2.0.2 - 2026-08-09

### Changed

- Redesigned both Windows batch launchers with a colorful, fully English build interface and clearer success and failure summaries.
- Added automatic selection of a compatible Node.js installation from `NVM_HOME` when the active Node.js version cannot run jsdom 30.
- Made `build_windows_no_log.bat` preserve the colorful console output without writing a persistent build log.
- Refreshed the Windows application icons and synchronized the diagnostics lockfile with the current application dependencies.

### Fixed

- Prevented Node.js 20 from reaching the frontend tests and failing with jsdom/undici runtime errors.
- Stripped ANSI color sequences from persistent logs while retaining colors in the interactive console.
- Preserved reliable exit codes and actionable error messages across both batch launchers.
- Exported loose Windows executables as one inseparable `artifacts\portable` folder so the required VST worker is not omitted when copying the application.
- Added a Git-tracked-file guard to automatic cleanup and preserved legacy build logs.

## 2.0.1 - 2026-08-09

### Changed

- Reworked the Windows build entry point with clear progress, streamed output, retained logs, and automatic cleanup after successful or failed builds.
- Improved the project README with a product-focused overview, architecture diagram, installation guidance, development commands, and troubleshooting.
- Simplified the GitHub release pipeline and added version consistency checks across frontend, Tauri, and Rust metadata.

### Fixed

- Bundled `OSCMidi.exe` and its required `vst-host-worker.exe` together in portable releases so the isolated host cannot be omitted accidentally.
- Added SHA-256 checksum generation for release packages and stricter validation that exactly one MSI is produced.
- Prevented restored CI caches from mixing MSI installers from older versions into the current release.

## 2.0.0 - 2026-08-09

### Added

- Isolated, supervised VST worker with authenticated control and MIDI channels.
- Worker heartbeat, bounded restart policy, crash/hang recovery, and worker diagnostics.
- Deterministic stress fixtures for normal, slow, blocked, and crashing worker behavior.
- Recoverable UI startup state and a top-level interface error boundary.

### Changed

- Made isolated worker hosting mandatory for every VST; the user-facing in-process switch and fallback were removed.
- Stabilized hot switching, state restoration, editor lifecycle, MIDI batching, and audio metrics.
- Reduced Tauri permissions and runtime plugins to the dialog APIs used by the interface.
- Updated React Router to 7.18.2, including the upstream RSC security correction.
- Updated Windows package, Rust crate, frontend package, and release metadata to 2.0.0.
- Refined spacing, focus targets, reduced-motion behavior, error feedback, and destructive-action confirmation.

### Fixed

- Prevented plug-in crashes and blocked destructors from terminating the desktop application.
- Corrected MMCSS handling, audio startup ordering, state persistence, VST editor sizing, and ASIO hot reloads.
- Preserved critical MIDI note-off/reset events under queue pressure.
