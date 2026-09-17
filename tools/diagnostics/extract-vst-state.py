"""Copy an OSCMidi v3 state into a NEW isolated directory and extract rack state.

The directory contains private plugin state: do not commit it. The JSON summary
contains hashes and optional Splice preset metadata, not the native state.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import struct
import xml.etree.ElementTree as ET
import zlib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("state", type=Path)
parser.add_argument("directory", type=Path)
args = parser.parse_args()
data = args.state.read_bytes()
if len(data) < 41 or data[:5] != b"OSVS\x03":
    raise SystemExit("Expected OSCMidi v3 state")
size = struct.unpack_from("<I", data, 5)[0]
if size > 256 * 1024 * 1024 or len(data) != size + 17:
    raise SystemExit("Invalid state length")
body = data[9:9 + size]
if zlib.crc32(body) != struct.unpack_from("<Q", data, 9 + size)[0]:
    raise SystemExit("Invalid state checksum")
offset = 24
blobs = []
for _ in range(5):
    if offset + 4 > len(body):
        raise SystemExit("Truncated state")
    count = struct.unpack_from("<I", body, offset)[0]
    offset += 4
    if count > len(body) - offset:
        raise SystemExit("Truncated state blob")
    blobs.append(body[offset:offset + count])
    offset += count
native = blobs[4]
if len(native) < 4 or struct.unpack_from("<I", native)[0] > len(native) - 4:
    raise SystemExit("No supported rack native state")
summary = {"stateSha256": hashlib.sha256(data).hexdigest(),
           "nativeSha256": hashlib.sha256(native).hexdigest(),
           "stateBytes": len(data), "nativeBytes": len(native)}
meta_match = re.search(rb"<META\s[^>]*?/>", native)
if b"<SpliceINSTRUMENT>" in native and meta_match:
    # The opaque plugin payload can include non-XML bytes; only inspect META.
    meta = ET.fromstring(meta_match.group())
    summary["preset"] = {k: meta.get(k) for k in ("family", "name", "version")}
args.directory.mkdir(parents=True, exist_ok=False)
(args.directory / args.state.name).write_bytes(data)
(args.directory / "native-state.bin").write_bytes(native)
(args.directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
print(json.dumps(summary, indent=2))
