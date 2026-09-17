# Independent VST3/ASIO comparison

This developer-only console host uses JUCE instead of the application's rack,
CPAL and worker IPC layers. It is not shipped with the application. JUCE source
and build products remain outside the tracked source tree; follow JUCE's own
license for any redistribution.

Validated dependency: JUCE 8.0.15, commit
`91ad83ae34a81e0833b1a2b0866f54846370ae53`.

```powershell
git clone --depth 1 --branch 8.0.15 https://github.com/juce-framework/JUCE.git .cargo-target/juce-reference
cmake -S tools/diagnostics/juce-reference -B .cargo-target/juce-reference-build -G 'Visual Studio 18 2026' -A x64 -DJUCE_SOURCE_DIR="$((Resolve-Path .cargo-target/juce-reference).Path)"
cmake --build .cargo-target/juce-reference-build --config Release --parallel 4
```

Run `juce_reference.exe <vst3-path> <seconds> <memory-cap-MiB> <new-report.jsonl> [rack-native-state]`.
The host requires Voicemeeter AUX Virtual ASIO at 48 kHz / 512 samples. It uses
a fresh plugin instance, optionally restores the supplied native state, provides a 120 BPM
playing timeline, and sends 16-note chords (48–63, velocity 80) every 32 ms with
16 ms gates: exactly 500 Note-On/Off pairs per second of processed audio.

This reproduces the production stress pattern approximately, not byte-for-byte:
the production diagnostic uses IPC arrival timestamps and Windows sleep pacing,
whereas this host generates events on the audio sample clock. There is no claim
that the plugin's opaque default state is identical across hosts. Match an
explicit preset before treating a difference as proof of a host defect.

For a matched state, run `extract-vst-state.py <saved-v3-state> <new-directory>`.
This validates the state checksum, copies the OSCMidi state and extracts the
native component/controller payload. Give the directory's `native-state.bin`
to this host and its absolute directory path to `audio-soak.ps1 -StateDirectory`.
The production worker's opt-in `OSCMIDI_VST_STATE_DIR` redirects load/save state
only for that process. Without it, production automatically restores user state
even when the diagnostic does not explicitly supply a preset. Never describe
that case as a guaranteed factory-default instance.

The JUCE host stores round-trip state beside the private input, allowing preset
verification. Keep these state files out of Git. A `<report>.stop` file requests
graceful early termination, recorded as incomplete rather than passed endurance.

The output is attenuated by 24 dB and clipped to +/-0.95. The report records
private memory, submitted event counts, pre-attenuation peak and callback budget
exceedances each second. Exceeding the chosen memory cap releases held notes,
drains audio for three seconds, and exits unsuccessfully. A successful completion
means duration/memory/format/count/signal checks passed; inspect overBudget
separately. This is not a downstream stuck-note or physical latency measurement.

No plugin memory instrumentation is attached. Neither passing this comparison
nor reproducing growth in it proves that the production application is free of
defects. The comparison narrows the host-specific part of the investigation.
