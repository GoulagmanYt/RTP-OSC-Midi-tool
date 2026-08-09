# Phase 2: isolated VST worker

## Runtime boundary

OSCMidi always launches one `vst-host-worker.exe` process for the selected
instrument. The worker owns the
VST DLL, CPAL/ASIO stream, audio callback, native editor window, parameter
controller, and state restoration. The main Tauri process never loads that
plug-in and never silently falls back to in-process hosting.

## IPC and authentication

Each launch creates a new session UUID, a 256-bit token, and two unpredictable
Windows named-pipe paths before the child is spawned:

- a full-duplex control pipe using versioned, length-prefixed JSON frames;
- a one-way binary MIDI pipe with sequence, QPC timestamp, length, and three
  MIDI bytes.

The first worker frame must authenticate the session ID and token and match both
the control protocol and host ABI versions. Control frames are limited to 1 MiB
and MIDI frames have a fixed 20-byte representation. The original QPC timestamp
is converted into message age in the worker so phase 1 stale-message policy is
still effective across the process boundary.

Supported control operations are `Load`, `OpenEditor`, `CloseEditor`,
`ListParameters`, `SetParameter`, `SetGain`, `SetLimiter`, `Panic`, `Stop`, and
`Ping`. State is saved by the serialized stop/reload transition before the
worker exits.

## Supervision

The worker emits a heartbeat every 500 ms containing the audio/VST metrics.
Three missed heartbeats mark it as hung. Process exit and pipe failure are also
detected. OSCMidi then closes the MIDI route, terminates the child, and permits
three automatic restarts in a rolling 60-second window before leaving the
runtime in `Faulted` for manual action.

The child is assigned to a Windows job configured with
`KILL_ON_JOB_CLOSE`, when the host environment permits it, so it cannot remain
orphaned after the main application exits. Pipe closure remains a second exit
path.

Runtime status and the one-second metrics stream expose worker state, restart
count, last exit, peaks, XRuns, MIDI drops/queue age, callback deadlines, DSP
percentiles, MMCSS, and power-throttling status. Closing the native editor from
its own title bar is relayed back to the main UI.

## Build and packaging

Tauri bundles the worker as an external binary. `tauri:dev` and `tauri:build`
run `scripts/prepare-vst-worker.ps1` automatically. The script builds the
matching debug or release worker and copies it using Tauri's required target
triple suffix. Generated sidecar executables are ignored by Git.

Manual preparation is also available:

```powershell
npm run prepare:vst-worker:dev
npm run prepare:vst-worker:release
```

## Automated validation

`vst-worker-fixture.exe` implements deterministic ready, native-crash, and hang
modes. The supervisor integration test verifies:

- authenticated connection and load/ready negotiation;
- 10,000 queued MIDI messages through the dedicated pipe;
- graceful stop;
- survival of an aborting child process;
- hang detection inside the 1.5-second heartbeat budget;
- three restarts followed by `Faulted`;
- handshake with the real worker binary and clean rejection of an invalid VST.

## Real-plugin acceptance still required

A manual smoke campaign was validated on 2026-08-04 with Splice INSTRUMENT
VST3, Keyzone Classic VST2, Voicemeeter AUX Virtual ASIO at 48 kHz/512,
worker-owned editors, live VST switching, deliberate worker termination, bridge
stop, and bridge restart. The host application remained available across the
injected worker failure.

For each release, run the build with Splice, Keyzone, and Upright Piano on the
target Voicemeeter ASIO device. The required campaign remains 30 minutes at
48 kHz/512, repeated editor operations, preset changes, zero driver XRuns, zero
stuck notes, and no consecutive deadline miss. Also terminate the worker from
Task Manager once and confirm that OSCMidi stays responsive and reports or
restarts the failed worker.
