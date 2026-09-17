# Windows heap diagnostic

This optional tool attaches to an explicitly selected, owned diagnostic worker.
Use a separate Python environment with `frida==17.18.0`. It does not change the
worker binary. Instrumentation can disturb or crash the process: these runs
must never be counted as clean audio latency or endurance evidence.

```
python heap-profile.py --pid <owned-worker-pid> --seconds 180 --output <new-file.jsonl>
```

Start `audio-soak.ps1` first and obtain the worker PID from that run's process
samples (which restrict child discovery to the owned diagnostic). Keep the
wrapper's memory guard enabled. Output uses exclusive creation to preserve
earlier evidence. Do not attach to an unrelated application or worker.

The native hooks track Win32 `HeapAlloc`, `HeapFree` and `HeapReAlloc` requests
of at least 8 KiB, using fixed-capacity storage. Reallocations remove the old
entry, track the successful replacement, or restore the old entry on failure.
Nested heap calls are excluded to avoid double counting a realloc's internal
allocation. Shrinking below the size threshold stops tracking that block.
Failed frees restore their entry. Native snapshots hold the same lock as hooks.
No JavaScript callbacks or stack unwinding occur on intercepted heap calls.

The `lost` counter must remain zero. This is still a partial allocation profile:
pre-attachment allocations, small blocks, direct Rtl/VirtualAlloc calls and bulk
HeapDestroy reclamation are not fully accounted for. Snapshots can include
in-flight operations. Caller attribution is a module offset, not a source-level
leak diagnosis or proof that an allocation occurred on the audio thread.

## Controlled verification

Run `heap-profile-fixture.py` in a separate console, attach to its printed PID,
then press Enter in the fixture console. Allow at least 38 seconds of profiling.
The FFI caller's live-byte sequence should be 16384, 32768, 0, 16384, 16384, 0
(individual phases can appear more than once due to the five-second sampling).
The fixture exercises allocate, grow, shrink below threshold, grow again,
failed resize and free. Both processes must exit successfully and `lost` must
be zero. The fixture also detects nested-allocation double counting.

See [Frida's native callback documentation](https://frida.re/docs/javascript-api/).
