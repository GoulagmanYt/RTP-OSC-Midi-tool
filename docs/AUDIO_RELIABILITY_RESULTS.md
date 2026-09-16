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
The original raw reports were written under `.cargo-target/splice-asio-*.json`.
Those untracked files were no longer present when the workspace was revisited;
the recorded summaries above remain, but the original raw reports are unavailable.

## Extended run

A subsequent 100,000-note run on the same rig and optimized audio worker lasted
204.029 seconds. All 100,000 Note-On and 100,000 Note-Off submissions completed:
zero rejected submissions, audio/supervisor MIDI drops, xruns, over-budget
callbacks, lock misses, route changes or worker restarts were reported. The queue
peaked at 16 and drained to zero. Signal was observed (peak approximately 0.086).
The final DSP p99 estimate was 3,250 us, maximum callback 7,679 us, maximum sampled
MIDI age 15,990 us, and graceful stop 19 ms. Both diagnostic checks passed.
Its original report was `.cargo-target/splice-asio-100000.json` (also unavailable
after the build-directory cleanup).

This extends the observed successful total to 130,000 notes. It exercises worker
IPC/audio, not the new RTP journal or SysEx paths, and retains all measurement
limitations above. Three minutes is not a long-duration soak or leak proof.

## Verification after audio and lifecycle fixes

The worker and diagnostic were rebuilt from code commit
`e4803dd29ae37690024a4e8cfda98b7078d94b73`, which includes audio fix `e2b5f12`.
A fresh 10,000-note run completed in 25.596 seconds with the same Splice/ASIO rig,
explicit Splice VST3 path and requested/actual 512-frame buffer. The complete
[raw report](measurements/splice-asio-audio-fixes.json) is tracked in this repository.
The measured worker executable SHA-256 was
`86B6EA78C39F19D496A617052CDA9DD6C5F07971BE013437F6B6F8E4047F2F83`.

| Measurement | Result |
| --- | ---: |
| Note-On / Note-Off submissions | 10,000 / 10,000 |
| Rejected submissions / supervisor drops / audio drops | 0 / 0 / 0 |
| Xruns / over-budget callbacks / lock misses / restarts | 0 / 0 / 0 / 0 |
| Maximum / final MIDI queue depth | 16 / 0 |
| DSP p99 estimate / maximum callback (us) | 3,500 / 8,673 |
| Maximum periodically observed MIDI age (us) | 11,605 |
| Backend-reported latency (us) | 21,333 |
| Peak signal amplitude | 0.0648 |
| Stop duration, including session-task completion (ms) | 106 |

Delivery and timing checks passed. The single emergency reset is the harness's
intentional final panic; observations during note transmission reported none.
No worker or diagnostic process remained after completion. Stop now waits for
session-task completion, so its duration is not directly comparable to the older
acknowledgement-only measurements. This run validates the latest audio/IPC code
under the stated load, with the physical latency and voice-accounting limitations
already described above.


## Extended validation and memory investigation (2026-09-16)

The continuous high-rate run using worker SHA-256
`D53C84630E6A0B0DA110522FB0B114E024C2C5E8157E7E460D13BD6E14530889`
(built from `01ac64d`) was stopped deliberately after 515 seconds, including
startup and shutdown. It submitted 255,760 matched Note-On/Note-Off pairs with no
reported audio drops, xruns, over-budget callbacks, lock misses or worker restarts.
This is **not a passed two-hour soak**: worker private memory reached
4,713,865,216 bytes at 502 seconds and was still growing during continuous input.
The [interrupted report](measurements/splice-asio-final-soak-20260916.json),
[resource samples](measurements/splice-asio-final-soak-20260916-processes.jsonl)
and executable provenance retain the failure rather than replacing it with a
shorter successful run. The software timing observations do not establish
physical audio latency or prove third-party voice release.

A separate control used the same worker, 60 seconds of MIDI followed by 120
seconds without further MIDI while audio remained running. All 30,384 submitted
pairs were accepted, delivery/timing checks passed and no restart occurred.
Worker private memory fell after the active phase and stayed around 735 MB during
idle. See the [control report](measurements/splice-memory-active-idle-20260916.json)
and [samples](measurements/splice-memory-active-idle-20260916-processes.jsonl).
This shows load-dependent retention; it does not identify the allocating module
or establish a safe upper bound under uninterrupted stress. WPR heap tracing was
unavailable to this process because Windows denied enabling it; no system tracing
settings were changed. The VST creation path already executes on the main UI
thread, so moving its outer lifecycle task is not a justified memory fix.

The diagnostic now supports explicit active/idle cycles and records their timing.
Its wrapper samples only its own process and immediate children, and requests a
graceful stop if worker private memory exceeds the configured bound. This is a
test containment measure, not a production memory fix or automatic worker restart.
A passed cyclic workload must not be reported as a passed continuous workload.


### Completed RTP endurance

The [one-hour RTP report](measurements/rtp-soak-20260916.json) passed after
3,600.797 seconds: 3,573 same-SSRC reconnections, 1,143,360 matched Note-On/Note-Off
pairs, 3,573 released/rebound socket pairs, and zero callback overflows. The
bidirectional batch round-trip observations (71,460 samples) were p50 83 us,
p95 138 us, p99 188 us and maximum 1,028 us. These are software loopback batch
measurements, not physical MIDI or audio latency. The process resource history
is retained alongside executable provenance. This executable was built before
the independent Chapter Q transport-comparison correction, which is covered by
the subsequent vendor regression suite.

### Cyclic memory failure and targeted profiling

The uninstrumented cyclic follow-up also failed the memory bound: the wrapper
requested graceful stop when private memory exceeded 2 GiB after about 321
seconds. It submitted 115,728 matched pairs with zero reported drops/xruns and
no restart, but did not complete its requested two hours. See the
[report](measurements/splice-cyclic-final-20260916.json) and resource samples.
A 60-second active / 30-second idle duty cycle is therefore not a workaround for
the accumulation.

A separate native HeapAlloc/HeapFree profile completed its 180-second diagnostic
without worker restart or reported delivery loss. It attributed very high gross
allocation traffic to `Splice INSTRUMENT.vst3+0x20f1c44` in installed version 2.4.2.
The [allocation trace](measurements/splice-heap-allocations-20260916.jsonl) covers
requests of at least 8 KiB made after attachment; it excludes pre-existing blocks
and does not account for HeapReAlloc. It is useful for locating allocation churn,
not proof of exact leaked bytes. Its instrumentation also excludes this run from
clean latency evidence. The raw [audio report](measurements/splice-heap-profile-20260916.json)
and process samples are retained. Most observed allocated bytes were freed;
allocator retention and outstanding plugin allocations still need to be separated.


### Windows allocator comparison

A copy of the worker was tested with only its manifest changed to request
Windows SegmentHeap; no plugin or DSP machine code was changed. This did not
stabilize continuous-load memory. Private memory again exceeded 2 GiB, triggering
the diagnostic stop before the requested eight minutes completed. The
[experiment provenance](measurements/splice-segment-experiment-20260916.json)
and [run report](measurements/splice-segment-continuous-20260916.json) preserve the
comparison. SegmentHeap is therefore not being adopted as a production fix.
