# OSCMidi v2 architecture

This document describes the production architecture compiled for Windows.

## Runtime boundaries

```text
React / Tauri desktop process
  ├─ configuration and diagnostics
  ├─ local MIDI, RTP-MIDI, OSC routing
  └─ AudioSupervisor
       ├─ authenticated control pipe (length-prefixed JSON)
       └─ bounded binary MIDI pipe
            ↓
       vst-host-worker.exe
         ├─ VST2/VST3 instance
         ├─ CPAL / ASIO or WASAPI stream
         ├─ real-time audio callback
         └─ native plug-in editor window
```

The desktop process never loads third-party VST DLLs. A plug-in crash, access violation, or blocked native destructor is contained in the worker. The supervisor monitors heartbeats, applies a bounded restart policy, and reports the last worker exit to the frontend.

## Backend layout

```text
src/
|-- main.rs                 # Tauri bootstrap and command registration
|-- application/            # Application services
|-- audio/                  # Worker-owned audio/VST engine
|-- bridge/                 # Prioritized MIDI routing pipeline
|-- vst_worker/             # IPC protocol and process supervision
|-- bin/vst_host_worker.rs  # Isolated production worker
|-- plugin_probe*.rs        # Timed, isolated plug-in probing
|-- reliable_playback.rs    # Deterministic recovery fixtures
|-- rtp*.rs                 # RTP-MIDI server and discovery
|-- tauri/commands.rs       # Public frontend command boundary
|-- config.rs               # Versioned YAML configuration
`-- types.rs                # Serialized frontend/backend contracts
```

## Design constraints

- Windows x64 is the only supported target.
- One instrument plug-in owns the configured output device at a time.
- Audio callbacks use bounded/preallocated queues and never wait on frontend work.
- Note-off, sustain-off, and reset messages remain recoverable under queue pressure.
- Control messages are versioned and bounded; each worker session uses unpredictable pipe names and a random authentication token.
- Plug-in scanning runs out of process with a per-candidate timeout and cached class identity.
- Frontend commands preserve the v1 API shape while lifecycle operations execute asynchronously.

## Shutdown and recovery

The supervisor closes MIDI input before asking the worker to stop. A cooperative worker saves state and releases its audio stream; a blocked worker is terminated by the Windows job object. Worker loss leaves the Tauri UI responsive and the bridge returns to a coherent faulted/stopped state instead of silently falling back to in-process hosting.

## Required validation

```powershell
npm run lint
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features
```

Release acceptance additionally requires a manual ASIO/VST campaign because real driver timing and third-party native editors cannot be represented faithfully by unit tests.
