# MIDI/RTP reliability changes

This audit and implementation apply to the local working tree, including the
existing audio/EQ changes. Those changes were preserved. No deployment or release
was performed.

## Patch boundaries

1. **Wire parsing:** validate complete packets before dispatch, bound delta-time
   decoding, preserve running status across real-time messages, normalize
   velocity-zero Note-On, fix 14-bit byte order and Z-flag encoding, and reject
   oversized outgoing batches. Packet errors propagate without panicking.
2. **Sessions and timing:** correlate invitations by token and endpoint, validate
   session version and clock count, emit departure events from all removal paths,
   expire invitations and both peer roles, and join registered background tasks.
   CK exchanges now supply monotonic deadlines with bounded integer rate
   compensation. Event deltas accumulate modulo 2^32.
3. **Bridge and destinations:** validate injected frames, use nonblocking fanout,
   invalidate old output generations after a fault, bound each routing batch,
   and track active external MIDI notes in the destination's owning thread.
   Device/filter changes flush notes; hotplug loss triggers recovery. OSC send
   errors propagate, failed sustain sends are not cached, and telemetry cannot
   wait on the activity lock in the routing path.
4. **Worker and exit:** emergency reset travels in the ordered MIDI pipe. Both
   its upstream backlog and the callback's pre-reset ring contents are discarded.
   Tauri exit waits for bridge/audio shutdown, and an RTP session started before
   the bridge is also stopped.
5. **Session ownership follow-up:** only public session handles own the
   cancellation guard. Dropping a value clone does not cancel surviving owners,
   while dropping the last owner cancels background contexts. Graceful stops
   serialize, join reception and maintenance tasks before draining peers, clear
   pending invitations, and prevent invitations on stopped sessions. BY uses
   best-effort nonblocking UDP sends so socket pressure cannot delay shutdown.
   Sequence history lives inside each participant, eliminating a separate map
   allocation and mutex on reception. One listener guard covers each packet;
   peer-state locks are released before callbacks. Reconnecting the same SSRC
   resets sequence history, verified through UDP injection.
6. **Regression harness:** malformed byte campaigns, real UDP session injection,
   sequence gaps/rollover/late packets, timestamp vectors, real-time interleaving,
   10,000 matched note lifecycles through bounded queues, output overflow,
   ordered IPC reset, callback backlog invalidation, and OSC byte alignment and
   round trips. Session regressions also verify retained clones communicate
   after the original is dropped, final-owner drop releases ports, and concurrent
   graceful stops both wait for registered tasks.

## Interfaces and runtime policy

- Worker protocol **8** requires the matching worker executable. A one-byte
  System Reset on the private MIDI pipe requests a callback reset; it is not
  passed to the instrument as an ordinary channel message. Rebuild both packaged
  worker architectures before creating a release.
- The vendored `TimestampedMidiMessage` includes a monotonic `deadline`.
  `StreamFaultEvent` identifies malformed traffic from an established endpoint
  so a corrupt final Note-Off does not require another packet to trigger recovery.
- Public bridge injection reports invalid input and queue exhaustion as errors.
  Audio output metrics count accepted submissions, not failed attempts or proof
  of physical playback.
- RTP scheduling uses a preallocated 4,096-entry heap, with stable ordering for
  equal deadlines. The fixed 3 ms delay was removed. Unsynchronized peers use
  arrival time plus intra-packet deltas until a clock exchange completes.
  Messages more than 10 seconds ahead are rejected, not played prematurely.
- Pending invitations expire after 30 seconds and are capped at 256; connected
  peers are capped at 128. Both session roles expire after 60 seconds without a
  valid synchronization packet, checked on the 10-second maintenance cycle.
- The conservative loss policy remains: a proven forward sequence gap triggers
  a reset and late/duplicate packets are discarded. There is no added sequence
  reorder delay. This preserves the latency policy at the cost of interrupted
  notes when the network reorders packets.
- A reset intentionally releases all destinations rather than attempting to
  reconstruct musical intent from missing messages. Queue pressure can still
  drop events; reset generations prevent replay of obsolete queued Note-Ons.
- `midi-types` 0.2.1 is vendored with two corrected assertions for valid value
  127. See `vendor/midi-types/PATCHES.md`; all three Cargo roots use this patch.

## Validation

Run from the repository root with the installed CMake and libclang on the process
environment. For tests/checks without packaged sidecars, use the same
`TAURI_CONFIG={"bundle":{"externalBin":[]}}` override as the worker preparation
script. This changes build inputs only, not the tracked application config.

```text
cargo test --manifest-path vendor/rtpmidi/Cargo.toml --locked --tests
cargo clippy --manifest-path vendor/rtpmidi/Cargo.toml --locked --tests -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo fmt --manifest-path vendor/rtpmidi/Cargo.toml -- --check
npm run lint
npm test
npm run build
```

Frontend validation used the installed Node 26 runtime; the machine's default
Node 20 does not satisfy the existing Vite/jsdom dependency requirements.

## Explicit limits

This is a reliability pass over implemented routes, not a certification of full
RFC 6295/OSC support. Recovery-journal reconstruction,
general OSC reception/bundle scheduling, and application-level outbound RTP
routing remain unsupported. Complete RTP SysEx is forwarded to local MIDI; its
existing untimestamped library event is dispatched at arrival time. Segmented
SysEx is reassembled per participant in a preallocated 64 KiB payload buffer;
only completed messages are delivered. Cancellation, corruption, packet loss,
non-real-time interruption, and a 10-second inter-fragment timeout discard the
partial command. Missing fragments are not reconstructed from journals. The
F5 dropped-EOX representation is normalized to a completed SysEx for MIDI APIs. Channel
voice messages use the synchronized scheduler.

The 10,000-note regression exercises the actual bounded enqueue/fanout functions
and destination note tracking in a headless harness. It does not replace a
hardware/driver test. The existing ASIO/VST tests requiring installed instruments
remain opt-in. No hard sub-millisecond or zero-network-loss claim follows from
these results: driver timing, Windows wake-up jitter, UDP delivery, and physical
disconnect recovery require measurements on the intended rig.

Protocol references: [RFC 6295](https://www.rfc-editor.org/rfc/rfc6295),
[Apple MIDI Network Driver Protocol](https://developer.apple.com/library/archive/documentation/Audio/Conceptual/MIDINetworkDriverProtocol/MIDI/MIDI.html),
and [OSC 1.0](https://opensoundcontrol.stanford.edu/spec-1_0.html).
