import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

TOOLS = Path(__file__).resolve().parents[1]

class DiagnosticToolTests(unittest.TestCase):
    def extract(self, root, metadata):
        native_xml = b"<SpliceINSTRUMENT>" + metadata
        native = struct.pack("<I", len(native_xml)) + native_xml
        body = bytes(24) + bytes(16) + struct.pack("<I", len(native)) + native
        state = root / "fixture.state"
        state.write_bytes(b"OSVS\x03" + struct.pack("<I", len(body)) + body + struct.pack("<Q", zlib.crc32(body)))
        output = root / "extracted"
        result = subprocess.run([sys.executable, str(TOOLS / "extract-vst-state.py"), str(state), str(output)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((output / "native-state.bin").read_bytes(), native)
        return json.loads((output / "summary.json").read_text())

    def test_quoted_xml_terminator_preserves_native_data_and_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = self.extract(Path(directory), b'<META family="Synth" name="A/>B" version="1"/>')
            self.assertEqual(summary["preset"]["name"], "A/>B")

    def test_malformed_metadata_does_not_prevent_binary_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = self.extract(Path(directory), b'<META name="A&B"/>')
            self.assertNotIn("preset", summary)

    def profile_with_stub(self, root, stub):
        (root / "frida.py").write_text(stub)
        env = dict(os.environ, PYTHONPATH=str(root))
        output = root / "profile.jsonl"
        result = subprocess.run([sys.executable, str(TOOLS / "heap-profile.py"), "--pid", "123", "--seconds", "1", "--output", str(output)], env=env, capture_output=True, text=True)
        return result, output

    def test_failed_attach_leaves_output_available_for_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            result, output = self.profile_with_stub(Path(directory), 'def attach(pid):\n    raise RuntimeError("No such process")\n')
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_detach_during_snapshot_is_handled(self):
        stub = """class InvalidOperationError(Exception): pass
class Session:
    def on(self, event, callback): self.callback = callback
    def create_script(self, source): return Script(self)
    def detach(self): pass
class Script:
    def __init__(self, session): self.session = session; self.exports_sync = self
    def on(self, event, callback): pass
    def load(self): pass
    def snapshot(self):
        self.session.callback()
        raise InvalidOperationError("detached")
def attach(pid): return Session()
"""
        with tempfile.TemporaryDirectory() as directory:
            result, output = self.profile_with_stub(Path(directory), stub)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(output.exists())

if __name__ == "__main__":
    unittest.main()
