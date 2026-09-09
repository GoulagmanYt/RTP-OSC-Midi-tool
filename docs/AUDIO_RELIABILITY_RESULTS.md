# Splice / Voicemeeter reliability measurements

Measured locally on Windows on 2026-09-09 with Splice INSTRUMENT VST3 x64,
Voicemeeter AUX Virtual ASIO, 48,000 Hz, the optimized production isolated worker,
limiter enabled and output gain -24 dB. MIDI was injected into the worker IPC in
groups of 16 notes, with matching releases after 10 ms and another 10 ms pause.
Windows timer scheduling can extend these pauses. No compilation ran during the
measurements. Persistent audio configuration was not changed by the harness.

| Measurement | First run | Smaller buffer requested | Final diagnostic run |
| --- | ---: | ---: | ---: |
| Requested buffer (frames) | 512 | 128 | 512 |
| Actual buffer (frames) | 512 | 512 | 512 |
| Note-On / Note-Off submitted | 10,000 / 10,000 | 10,000 / 10,000 | 10,000 / 10,000 |
| Rejected submissions / audio MIDI drops | 0 / 0 | 0 / 0 | 0 / 0 |
| Reported xruns / callbacks over budget | 0 / 0 | 0 / 0 | 0 / 0 |
| DSP p99 histogram estimate (us) | 3,500 | 3,500 | 3,500 |
| Maximum callback duration (us) | 7,901 | 7,731 | 7,451 |
| Maximum observed MIDI queue age (us) | 15,958 | 11,967 | 11,965 |
| Maximum MIDI queue depth / final depth | 16 / 0 | 16 / 0 | 16 / 0 |
| Backend-reported latency (us) | 21,333 | 21,333 | 21,333 |
| Worker restarts | 0 | 0 | 0 |
| Graceful stop duration (ms) | 20 | 6 | 5 |

Audio signal was observed in all three runs. The final run lasted 25.209 seconds,
reported both delivery and timing checks passed, and recorded zero supervisor
MIDI drops. Its peak amplitude was approximately 0.075, falling to approximately
0.000003 after emergency reset and a three-second settling period.

These are telemetry observations, not a measurement of physical MIDI-to-speaker
latency or proof that every voice sounded correctly. The final panic can hide a
stuck voice; matching note accounting is separately covered by bridge regression
tests. The MIDI-age figures are periodically sampled, not exhaustive worst-case
latency bounds. At 512 frames the block period is 10.667 ms; the requested 128-frame
setting did not change Voicemeeter's actual buffer. No sub-millisecond or
zero-loss-under-all-conditions claim is supported. An interrupted debug run was
excluded from these results.

## Reproduction

Build with the repository's Windows CMake/libclang environment available:

```powershell
$env:TAURI_CONFIG = '{"bundle":{"externalBin":[]}}'
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --features vst-worker-binary --bin vst-host-worker
cargo build --manifest-path tools/diagnostics/Cargo.toml --locked --bin audio_reliability
$env:OSCMIDI_VST_WORKER_X64_PATH = (Resolve-Path .cargo-target/release/vst-host-worker.exe).Path
& .cargo-target/debug/audio_reliability.exe --notes 10000 --buffer 512 --report .cargo-target/audio-reliability.json
```

The diagnostic uses the saved VST selection and defaults to Voicemeeter AUX
Virtual ASIO at 48 kHz. `--vst <path>` and `--device <ASIO name>` override those
selections. It does not save the requested device/buffer settings. The production
worker may checkpoint instrument state during shutdown. Use the explicit worker
path so a stale debug or packaged worker is not measured accidentally.

The JSON includes actual/requested buffers, signal presence, telemetry samples,
final queue state, restarts and shutdown errors. Failed delivery or timing checks
produce a nonzero exit code. Signal presence is reported separately because an
unconfigured or intentionally silent instrument can process MIDI correctly.
Reports from the measured runs remain under `.cargo-target/splice-asio-*.json`.
