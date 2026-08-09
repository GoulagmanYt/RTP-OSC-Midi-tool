# Changelog

All notable changes to OSCMidi are documented here.

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
