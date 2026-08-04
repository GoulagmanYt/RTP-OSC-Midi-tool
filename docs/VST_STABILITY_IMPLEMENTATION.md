# VST stability implementation status

## Implemented in-process hardening

- Windows scheduling uses `ABOVE_NORMAL_PRIORITY_CLASS`; MMCSS error 1552 is
  treated as a driver-owned registration and never falls back to
  `TIME_CRITICAL`.
- Audio lifecycle operations are serialized through the states `Stopped`,
  `Loading`, `Running`, `Stopping`, and `Faulted`.
- Hot VST reloads retain the active CPAL/ASIO device handle: the old callback is
  paused and removed, then the new runtime is attached without unloading and
  re-enumerating the exclusive ASIO driver. Concurrent reload requests are
  rejected by the backend and disabled in the UI while a transition runs.
- VST2 and VST3 creation, initialization, controller metadata, state
  restoration, editor lifecycle, and destruction are dispatched on the main
  UI thread. Audio `process()` remains on the ASIO callback thread. The legacy
  VST2 editor path calls `effEditGetRect` only after `effEditOpen`, then applies
  the reported client size with DPI-aware Win32 framing. Invalid sizes retain
  the 800x600 compatibility container.
- ASIO driver enumeration is dispatched on the registered Windows UI/STA
  thread even though Tauri lifecycle commands are asynchronous. This preserves
  Voicemeeter driver compatibility without running the full load operation on
  the UI thread.
- Bridge shutdown runs its blocking joins outside Tokio workers and stops MIDI
  producers before the audio consumer. Synchronous RTP lifecycle paths use a
  runtime-aware adapter, preventing the nested `block_on` panic that previously
  aborted release builds when the bridge was stopped.
- Release builds use panic unwinding so guarded Rust worker/audio panics can be
  contained and reported instead of unconditionally terminating OSCMidi.
- Splice INSTRUMENT and sforzando VST3 instances are quarantined after their
  state and stream have been closed. Their native destructors are not invoked
  in-process because they can block or crash the UI thread; Windows reclaims
  the retained memory when OSCMidi exits. Full reclamation requires phase 2
  process isolation.
- Plugin state is restored before `stream.play()`. State capture occurs only
  after the stream stops; disk writes happen after releasing the plugin lock and
  use atomic replacement.
- State identities include format, canonical path, and the cached VST UID. The
  filename-only legacy state is migrated only when the cache proves it is
  unambiguous.
- VST3 audio processing no longer takes the UI/state mutex. Parameter edits use
  bounded preallocated UI-to-audio transfer queues.
- VST3 event lists and parameter queues are preallocated before playback. MIDI
  conversion reuses a 512-event Rust buffer and MIDI plus audio processing cross
  FFI in one call. VST2 MIDI is sent as one preallocated event batch.
- A plugin-lock miss now ramps the last block to silence and ramps the next
  successful block back in; a block is never repeated indefinitely.
- MIDI is timestamped with a monotonic clock. Stale non-critical messages older
  than two measured periods are dropped while note-off and sustain-off are
  preserved.
- OSC and MIDI Thru run on independent bounded workers after the priority audio
  injection path. UI telemetry remains coalesced.
- VST probing runs in a separate process per candidate with a ten-second
  timeout. Cache entries include class UID, architecture, file timestamp, size,
  and host ABI. VST3 loading uses the cached UID without rescanning the parent
  directory.

## Observability

Runtime status and metrics now expose buffer period, declared plugin latency,
DSP last/P95/P99/max, deadline misses, MIDI queue depth/max/oldest age, lifecycle
state, and reserved worker status fields. `audioLatencyMs` remains a compatibility
estimate equal to buffer period plus declared plugin latency; driver latency is
reported as unknown in the UI.

The Audio page consumes the one-second runtime metrics stream instead of showing
only the last command-triggered status snapshot. It displays live MIDI throughput
and last-block DSP time. MIDI debug messages are sampled at one per second so a
song or stress run cannot turn file logging into a pipeline bottleneck.

The built-in stress test is bounded to 10,000 MIDI messages/s and its UI profile
uses 5,000 messages/s for five seconds. It sends balanced note-on/note-off pairs,
waits for queues to drain before reading counters, and exercises fan-out queues
without flooding external MIDI or OSC destinations. The former UI "End-to-End"
mode was hidden because it injected after RTP and therefore did not test RTP at
all; true RTP end-to-end testing remains external/phase 2 work.

## Phase 2 worker

Phase 1 is frozen in commit `e8de779`. The optional phase 2 worker is described
in `docs/VST_WORKER_PHASE2.md`. It is disabled by default and can be enabled from
the Audio page for real-plugin validation; no automatic fallback to in-process
hosting occurs when isolation is selected.

The 30-minute Splice/Voicemeeter release campaign still requires the target
audio device. The existing real-plugin smoke tests remain opt-in/ignored.

## Validation performed

- `cargo test --all-targets`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --release --all-targets`
- `npm run lint`
- `npm test`
- `npm run build`
