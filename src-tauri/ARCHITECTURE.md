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
       vst-host-worker-x64.exe
              |
              +-- authenticated shared memory (3 slots, one-buffer pipeline)
                     vst-host-worker-x86.exe
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
- VST3 uses the pinned Steinberg SDK `v3.8.1_build_84` at commit
  `3cdf9ca5d1f5b1b21e0a86832aa4abe55607bd96`; packaging rejects a different
  revision. VST2.4 is a legacy ABI compatibility layer, not an evolving SDK.
- The VST3 processor lifecycle is mirrored on shutdown, native component and
  controller state are authoritative, and normalized parameters are retained
  only as a recovery fallback.
- CPAL is pinned to `0.18.1`. WASAPI uses shared low-latency mode; ASIO buffer
  requests are preferences validated by the driver, and the active stream's
  actual buffer size and callback-to-playback latency are reported separately.
- A CPAL xrun increments telemetry without rebuilding. Device loss, ASIO reset,
  sample-rate/buffer invalidation, or a fatal backend error checkpoints VST
  state and faults the worker so the bounded supervisor restart policy can
  rebuild the complete stream safely.
- The 1 ms Windows timer request is scoped to the audio runtime and paired with
  `timeEndPeriod`; MMCSS registration remains scoped to the callback thread.

## Shutdown and recovery

The supervisor closes MIDI input before asking the worker to stop. State is keyed by `PluginId` and stored in a checksummed envelope containing the typed native VST2/VST3 state plus a normalized parameter fallback. Compatibility profiles that cannot safely use native VST3 state persist only parameters explicitly changed by the host, editor, program selection, or processor; read-only telemetry and untouched MIDI proxies are excluded. Unsafe legacy dense snapshots are archived rather than applied. Parameter edits are autosaved after a debounce; editor close, reload, and shutdown request a full checkpoint from the worker that owns the plug-in. A blocked worker is terminated by the Windows job object without replacing the last valid state file. Worker loss leaves the Tauri UI responsive and the bridge returns to a coherent faulted/stopped state instead of silently falling back to in-process hosting.

The Windows audio behavior is based on the official [ASIO SDK](https://www.steinberg.net/developers/asiosdk-open/), Microsoft [IAudioClient](https://learn.microsoft.com/windows/win32/api/audioclient/nn-audioclient-iaudioclient), [low-latency audio](https://learn.microsoft.com/windows-hardware/drivers/audio/low-latency-audio), [device invalidation recovery](https://learn.microsoft.com/windows/win32/coreaudio/recovering-from-an-invalid-device-error), [MMCSS](https://learn.microsoft.com/windows/win32/procthread/multimedia-class-scheduler-service), and [timer-period](https://learn.microsoft.com/windows/win32/api/timeapi/nf-timeapi-timebeginperiod) contracts, together with the [CPAL 0.18.1 changelog](https://github.com/RustAudio/cpal/blob/v0.18.1/CHANGELOG.md).

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
