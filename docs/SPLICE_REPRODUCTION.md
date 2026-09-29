# Splice INSTRUMENT memory growth under repeated MIDI chords

This is a local investigation report, not a support request that has been sent.
The evidence shows memory growth; it does not establish a source-level leak or
exclude all host, driver, preset and scheduling interactions.

## Configuration

- Windows x64; Splice INSTRUMENT VST3 2.4.17.
- Plugin SHA-256: `D4F9686268DEBCC7B71C2FF277AB056805EAB67F4C6E99B634E961003C9A2E0F`.
- Preset metadata: Autograph Grand / The Grand, version 1.0.1, saved modified state.
- Native component: 6,850 bytes, SHA-256
  `053d888826086c8b38149a892d40ce635fcc78be8662f46eeb09ba9d605dbde9`.
- Voicemeeter AUX Virtual ASIO, 48 kHz, 512 samples (10.667 ms block period).
- MIDI channel 1, pitches 48–63, velocity 80; repeated 16-note chords with short
  gates, approximately 500 matched Note-On/Off pairs per second. This is an
  intentionally demanding stress case, not a typical piano performance.

## Independent reproduction

The [JUCE reference host](../tools/diagnostics/juce-reference/README.md) uses JUCE
8.0.15 (`91ad83ae34a81e0833b1a2b0866f54846370ae53`) instead of the application's
rack/CPAL/worker IPC implementation. It schedules chords every 32 ms with 16 ms
gates on the audio sample clock. Production retains Windows sleep/IPC pacing,
so the scheduling is comparable rather than bit-identical.

Use [extract-vst-state.py](../tools/diagnostics/extract-vst-state.py) to copy the
saved v3 state into a new private directory. Pass its native-state.bin to the
reference host and its directory to `audio-soak.ps1 -StateDirectory`. The
reference host reads the state back before playback; the [state check](measurements/splice-juce-matched-20260917-state-check.json)
confirmed exact component-byte equality. Native preset payloads are kept local,
not included in this report or automatically transmitted.

The [matched JUCE control](measurements/splice-juce-matched-20260917.jsonl)
completed 15 minutes and 450,016 matched pairs, ending at 3,685,584,896 private
bytes (3.43 GiB). It recorded 32 callback budget exceedances; this is memory
reproduction evidence, not a clean timing pass. Production ran concurrently
during part of that control, as documented in [the results](AUDIO_RELIABILITY_RESULTS.md).

The preceding production run was manually stopped by the user after about
82 minutes. Its EOF/restart must not be called a spontaneous crash. Memory in
that run stayed much lower, showing that onset and magnitude vary between runs.

The September 18 single-host retry uses the same native payload, no independent
host or memory instrumentation concurrently, and a 6 GiB diagnostic guard.
Its outcome and final counters are recorded in AUDIO_RELIABILITY_RESULTS.md.
That guard is a safety choice, not a published Splice memory specification.

That retry reached 6,547,107,840 private bytes and stopped at 859.167 seconds
including startup/drain/shutdown, after 430,848 matched pairs. It reported zero
MIDI rejections/drops, xruns, callback budget exceedances and worker restarts.
The saved stop marker attributes the interruption to the memory guard. This
single-host result rules out the simultaneous JUCE comparison as a necessary
condition for observing the growth, but does not identify its internal cause.

## Allocation evidence and unresolved questions

A separate partial native profile accounted for HeapAlloc, HeapFree and
HeapReAlloc, excluding nested calls. A controlled allocation fixture verified
grow/shrink/failed-resize/free behavior. Tracked live allocations from a Splice
call site rose across idle pauses to approximately 102 MiB. This profile excludes
small and pre-attachment blocks and complete bulk-heap accounting; see
[profiling limitations](../tools/diagnostics/HEAP_PROFILING.md).

The publisher describes generally 300–500 MB per preset, higher for Legato,
but specifies neither a hard ceiling nor behavior at this note rate:
[technical information](https://support.splice.com/en/articles/12453251-technical-information-and-compatibility-for-splice-instrument).
The [change log](https://support.splice.com/en/articles/12295094-splice-instrument-change-log)
mentions a historical default voice-limit fix, but does not document the native
`a_voiceLimit=-1` value found in this saved preset. It has not been modified.

Questions for the plugin publisher:

1. Is continued growth for this fixed-pitch, fixed-velocity workload expected,
   and should the cache reach a documented bound?
2. What does `a_voiceLimit=-1` mean in this preset/version?
3. Is there a supported setting or update that bounds memory under repeated
   short notes without requiring periodic instance resets?

No plugin reset workaround, binary patch, artificial passing threshold or
source-level attribution is presented as a fix.
