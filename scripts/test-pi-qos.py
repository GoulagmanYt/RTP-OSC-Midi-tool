"""Exercise the real QoS shell script without root, nftables, or host changes."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


BASH = shutil.which("bash") or "C:/Program Files/Git/bin/bash.exe"
SOURCE = Path(__file__).with_name("setup-pi-qos.sh")
EXISTING = b"#!/usr/sbin/nft -f\nflush ruleset\n# existing firewall and NAT\ntable inet firewall { }\n"
MOCKS = r'''
id() { if [ "$#" -eq 1 ]; then echo 0; elif [ "${TEST_UID:-}" ]; then echo "$TEST_UID"; else return 1; fi; }
apt-get() { echo 'Unexpected package installation' >&2; return 99; }
systemctl() {
    printf 'systemctl %s\n' "$*" >> "$TEST_LOG"
    [ "${TEST_FAIL:-}" != enable ]
}
nft() {
    printf 'nft %s\n' "$*" >> "$TEST_LOG"
    if [ "$1" = -c ]; then
        [ "${TEST_FAIL:-}" != check ]
    elif [ "$1" = -f ]; then
        [ "${TEST_FAIL:-}" != apply ] || return 1
        cat "$2" >> "$TEST_APPLIED"
    fi
}
'''


class QosTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="oscmidi-qos-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.conf = self.root / "nftables.conf"
        self.conf.write_bytes(EXISTING)
        script = SOURCE.read_text().replace("/etc/nftables", self.root.as_posix() + "/nftables")
        self.script = self.root / "setup.sh"
        self.script.write_text(script.replace("set -euo pipefail", "set -euo pipefail\n" + MOCKS))

    def run_script(self, fail="", uid=""):
        env = dict(os.environ, TEST_LOG=(self.root / "commands").as_posix(),
                   TEST_APPLIED=(self.root / "applied").as_posix(), TEST_FAIL=fail, TEST_UID=uid)
        result = subprocess.run([BASH, self.script.as_posix()], env=env,
                                text=True, capture_output=True, timeout=20)
        return result

    def test_preserves_existing_firewall_and_is_idempotent(self):
        result = self.run_script(uid="123")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.conf.read_bytes().startswith(EXISTING), "Existing firewall config was overwritten")
        first_config = self.conf.read_bytes()
        fragment = self.root / "nftables.d/oscmidi-qos.nft"
        first_fragment = fragment.read_bytes()
        self.assertEqual(first_config.count(b'include "'), 1)
        self.assertIn(b"meta skuid 123", first_fragment)
        self.assertIn(b"add table inet oscmidi_qos", first_fragment)
        self.assertIn(b"flush table inet oscmidi_qos", first_fragment)
        self.assertNotIn(b"flush ruleset", first_fragment)
        self.assertNotIn(b"table inet filter", first_fragment)
        self.assertEqual(set(re.findall(rb"table inet (\w+)", first_fragment)), {b"oscmidi_qos"})
        self.assertEqual(re.findall(rb"\bchain (\w+)", first_fragment), [b"output"])
        self.assertEqual(self.run_script(uid="123").returncode, 0)
        self.assertEqual(self.conf.read_bytes(), first_config)
        self.assertEqual(fragment.read_bytes(), first_fragment)
        self.assertEqual((self.root / "applied").read_bytes(), first_fragment * 2)
        commands = (self.root / "commands").read_text()
        self.assertNotIn("--now", commands)
        self.assertNotIn("restart", commands)
        self.assertNotIn("reload", commands)
        self.assertTrue(commands.startswith("nft -c -f "), commands)

    def test_command_failures_leave_persistent_config_unchanged(self):
        for failure in ("check", "enable", "apply"):
            with self.subTest(failure=failure):
                result = self.run_script(fail=failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.conf.read_bytes(), EXISTING)
                self.assertFalse((self.root / "nftables.d/oscmidi-qos.nft").exists())

    def test_failed_update_preserves_previous_installation(self):
        self.assertEqual(self.run_script(uid="123").returncode, 0)
        fragment = self.root / "nftables.d/oscmidi-qos.nft"
        old_config, old_fragment = self.conf.read_bytes(), fragment.read_bytes()
        for failure in ("check", "enable", "apply"):
            with self.subTest(failure=failure):
                self.assertNotEqual(self.run_script(fail=failure, uid="456").returncode, 0)
                self.assertEqual(self.conf.read_bytes(), old_config)
                self.assertEqual(fragment.read_bytes(), old_fragment)

    def test_missing_config_and_missing_daemon_user(self):
        self.conf.unlink()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b'include "', self.conf.read_bytes())
        fragment = (self.root / "nftables.d/oscmidi-qos.nft").read_bytes()
        self.assertNotIn(b"meta skuid", fragment)
        self.assertNotIn(b"flush ruleset", fragment)


if __name__ == "__main__":
    unittest.main()
