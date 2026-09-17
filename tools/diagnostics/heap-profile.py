"""Attach explicitly to an owned diagnostic PID. Requires frida==17.18.0.

This is an intrusive diagnostic, never clean timing/endurance evidence.
Only blocks >=8 KiB observed through Win32 Heap APIs are accounted for.
The fixed table reports lost entries; nonzero lost invalidates live-byte totals.
"""
import argparse
import json
from pathlib import Path
import threading
import time

import frida

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--pid", type=int, required=True)
parser.add_argument("--seconds", type=int, default=180)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
if args.pid <= 0 or not 1 <= args.seconds <= 43200:
    parser.error("Positive PID and 1..43200 seconds required")
detached = threading.Event()
errors = []
with args.output.open("x", encoding="utf-8", buffering=1) as output:
    session = frida.attach(args.pid)
    session.on("detached", lambda *_: detached.set())
    try:
        script = session.create_script(Path(__file__).with_suffix(".js").read_text(encoding="utf-8"))
        def on_message(message, data):
            output.write(json.dumps({"unixTime": time.time(), "message": message}) + "\n")
            if message.get("type") == "error":
                errors.append(message)
        script.on("message", on_message)
        script.load()
        detached.wait(args.seconds)
        if not detached.is_set():
            script.exports_sync.snapshot()
    finally:
        session.detach()
if errors:
    raise SystemExit("Instrumentation errors recorded; profile is invalid")
