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


### Event-timeline correction: memory issue persists

Worker commit `2fe6f98` aligns Event.ppqPosition with the advancing ProcessContext
clock. Its [continuous-run report](measurements/splice-ppq-continuous-20260916.json)
records 89,936 matched submissions and no reported drops/xruns, over-budget
callbacks or worker restarts. Nevertheless, private memory exceeded 2 GiB and the
guard stopped the run after 186.811 seconds including startup/drain/shutdown.
It is not a passed two-hour endurance test. The correction is retained for its
VST3 timing semantics; memory behavior remains an independent unresolved issue.


### Current Splice version control

The official signed installer was downloaded and its VST3 2.4.17 payload extracted
into the ignored build directory. The installer was never run and the installed
2.4.2 copy in Program Files was not replaced. The [candidate provenance](measurements/splice-2417-candidate-20260916.json)
records installer signature status and both hashes. Version 2.4.17 is documented
in the [publisher's change log](https://support.splice.com/en/articles/12295094-splice-instrument-change-log).

The [2.4.17 control](measurements/splice-2417-continuous-20260916.json) produced
an audio signal and submitted 126,320 matched pairs with zero reported drops/xruns
or restarts. It nevertheless exceeded the same 2 GiB private-memory limit and
stopped after 257.140 seconds including startup/drain/shutdown. Updating the
plugin alone is therefore not a demonstrated fix for this stress case. This run
still predates the per-block MIDI offset-ordering correction being validated.


### Fixed MIDI block ordering: delayed but unresolved memory growth

Worker commit `b3da31a` uses one timestamp origin per MIDI batch and preserves
nondecreasing sample offsets without reordering FIFO messages. The
[continuous run](measurements/splice-ordered-midi-20260917.json) with installed
Splice 2.4.17 and Voicemeeter AUX ASIO produced audio and submitted 664,800
matched pairs with zero reported drops, xruns, over-budget callbacks or worker
restarts. Shutdown completed in 150 ms. All 152 application tests and Clippy
passed before the release build.

Private memory remained around 620–675 MiB for approximately 19 minutes, then
grew rapidly and exceeded the 2 GiB guard. The run stopped after 1,329.972
seconds including startup/drain/shutdown, not the requested two hours. A longer
initial plateau is not proof that the memory defect was fixed or of a causal
improvement; the ordering correction remains independently justified. The
[process samples](measurements/splice-ordered-midi-20260917-processes.jsonl),
[binary provenance](measurements/splice-ordered-midi-20260917-provenance.json) and
[rig identity](measurements/splice-ordered-midi-20260917-rig.json) preserve the
evidence. The retained stop marker records the memory-limit reason.

Rig identity collected during the September 17 run shows that the installed
plugin is now 2.4.17 (SHA-256 starts `D4F96862`). The agent did not run an
installer or replace Program Files. The installation changed since the earlier
2.4.2 measurements, so this is not a controlled same-plugin comparison against
those runs. Future wrapper provenance captures plugin version and hash
automatically instead of relying on a separately collected rig record.


### Native heap profile with realloc accounting

The follow-up [profile](measurements/splice-realloc-allocations-20260917.jsonl)
tracks Win32 HeapAlloc/HeapFree/HeapReAlloc using bounded native callbacks. Its
[controlled fixture](measurements/heap-profile-fixture-20260917.jsonl) verified
allocation, growth, shrink below threshold, failed resize and free, including
exclusion of nested allocator calls. No profile snapshot reported table loss
or script error. Tool source and limitations are in
[HEAP_PROFILING.md](../tools/diagnostics/HEAP_PROFILING.md).

Four 60-second active / 30-second idle cycles on installed Splice 2.4.17
submitted 120,864 matched pairs and produced audio. The diagnostic completed
in 368.630 seconds, with no reported worker restart or MIDI rejection and
115 ms shutdown. Although its telemetry reports no drops/xruns, this is an
instrumented run and is excluded from clean latency/endurance evidence.

Tracked live plugin allocations during successive pauses increased from zero
to approximately 46.45, 75.99 and 101.64 MiB. The final 106,579,232 bytes were
attributed to `Splice INSTRUMENT.vst3+0x2174ab4`; cumulative allocations at that
site were about 95.87 GB, most of which were freed. The
[summary](measurements/splice-realloc-profile-20260917-summary.json) preserves
exact counts and source revisions. The increasing live subset establishes
retention in observed allocation calls, rather than only an increase in gross
allocation traffic or Windows private bytes. It does not establish which
internal objects retain them, whether this is a cache or leak, or whether host
interaction triggers the behavior. Direct allocation APIs, small/pre-attachment
blocks and bulk heap destruction are outside complete accounting.

At that stage, the retention cause and two-hour endurance remained unresolved.
No plugin source-level fix or physical-loopback latency measurement was claimed.
The later independent-host comparison and user-confirmed interruption are
documented below. The production timing correction and successful RTP endurance
remain valid independently of these audio stress results.

### Matched-state comparison setup (September 17 follow-up)

Inspection found that production startup automatically restores saved plugin
state even when `supervisor.start(settings, None)` supplies no explicit fallback
path. Earlier wording about a fresh instance must not be read as proof of a
factory-default preset. The first independent JUCE exploration used an
unverified default, was stopped intentionally, and is not a matched-preset
comparison or completed endurance test. Its retained stop record explains this.

The subsequent control loads the extracted native state from an isolated copy
of the production v3 state. Splice identifies it as Autograph Grand / The Grand
1.0.1. JUCE returned the exact same 6,850 component bytes after restoration,
verified by SHA-256 in the state-check report. The production worker now has
an opt-in absolute `OSCMIDI_VST_STATE_DIR`; diagnostics use a private copy so
startup/shutdown no longer read/write the user's state during this comparison.
The wrapper records initial state hashes. This override is unused in normal
application operation. All 152 application tests and Clippy passed.

Independent host: JUCE 8.0.15, no production rack/CPAL/IPC code, same Splice
2.4.17 binary and Voicemeeter AUX ASIO at 48 kHz/512 frames. It sends the same
16-note pitches/velocity at 500 pairs/second on its sample clock; production
retains IPC/Windows pacing, so event timestamps are not bit-identical.
The new safety guard is 6 GiB, not a publisher memory limit or a passing
stability criterion. Initial concurrent operation is additional system load;
JUCE recorded callback budget exceedances when the second host started.
Neither comparison nor a higher safety cap alone certifies memory stability.

### Completed control and user-interrupted endurance (clarified September 18)

The matched-state JUCE control completed 900 seconds and 450,016 matched note
pairs. Its final private memory was 3,685,584,896 bytes (approximately 3.43 GiB),
below its 6 GiB safety cap. It recorded 32 callback budget exceedances. This
reproduces memory growth outside the production host with the exact same native
component state; it does not prove a leak, a normal cache plateau, or a clean
timing pass. Its `completed` flag is a duration/memory/count/signal check only.

The production run lasted 4,946.844 seconds including startup/drain/shutdown
and submitted 2,524,048 Note-On/Off pairs. On September 18 the user explicitly
confirmed that they manually stopped the worker. Consequently the recorded
control-channel EOF, one restart and 112 rejected submissions are observations
of an interrupted run, not evidence of an unexplained spontaneous crash.
The raw failure fields are preserved; the companion interruption record adds
the user's clarification rather than rewriting the telemetry as a pass.

Sampled worker private memory peaked at 1,262,587,904 bytes (about 1.18 GiB),
below the configured 6 GiB guard. The run reported zero xruns, but 76 callback
budget exceedances were already present before the interruption. These timing
observations remain valid and are not erased by the clarification. Initial
concurrent operation with JUCE is documented above.

**Remaining:** complete an uninterrupted two-hour endurance, assess the timing
exceedances and memory trajectory, and retain the limits of the Splice comparison.
There is no spontaneous worker-crash defect established by this particular run.

### September 18 single-host final attempt: memory guard reached

The [final attempt](measurements/splice-final-endurance-20260918.json) used the
same native state hash `51670769c2a0289fc6d1721a71d5e61fddc05f305072df2815f9759e74aa0310`
in a new isolated directory. There was no simultaneous JUCE host, build or heap
instrumentation. The requested two-hour duration was stopped by the wrapper's
6 GiB memory guard after 859.167 seconds including startup/drain/shutdown
(849.587 seconds of transmission). Peak sampled worker private memory was
6,547,107,840 bytes at 853.876 seconds, approximately 6.10 GiB.

The run submitted 430,848 matched pairs and produced audio. It recorded zero
rejected submissions, MIDI drops, xruns, callback budget exceedances, lock
misses and worker restarts. Before the final panic the queue was empty; maximum
callback time was 4,886 us and maximum plugin processing time 4,879 us. Shutdown
completed in 331 ms. These observations do not imply two-hour success: the
memory guard stopped the test early and `deliveryPassed` is consequently false.

The generic `external stop requested` field is attributable to the saved
[guard marker](measurements/splice-final-endurance-20260918.json.stop), which
states that worker private memory exceeded the diagnostic limit. This is
distinct from the user's manual termination of the September 17 run.

The earlier 82-minute run had a roughly 1.0–1.1 GiB plateau from approximately
30 to 80 minutes. The new growth with the same native state shows variability;
neither that earlier plateau nor the public typical-memory estimate establishes
stability for every run. No speculative change to opaque voice-limit settings,
periodic reset workaround, or claim that all memory is leaked was made.

**Result:** the repository changes and automated checks are delivered, but the
requested two-hour Splice memory validation is blocked by a reproducible stress
outcome also exhibiting growth in an independent host. The remaining cause is
not established sufficiently to justify another production patch. The local
[reproduction report](SPLICE_REPRODUCTION.md) records configuration, evidence,
limitations and questions for the publisher; no external message was sent.
