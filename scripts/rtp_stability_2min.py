#!/usr/bin/env python3
import argparse
import json
import re
import socket
import subprocess
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Dict, Optional


REPO_ROOT = Path(__file__).resolve().parents[1]
PLINK = Path(r"C:\Program Files\PuTTY\plink.exe")
PI_HOST = "pianoledvisualizer.local"
PI_USER = "plv"
PI_PASS = "visualizer"


def now_stamp() -> str:
    return datetime.now().strftime("%Y-%m-%d_%H-%M-%S")


def run_cmd(cmd, timeout: Optional[int] = None, cwd: Optional[Path] = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )


def run_pi_python(script: str, timeout: int) -> subprocess.CompletedProcess:
    remote = "python3 - <<'PY'\n" + script + "\nPY"
    cmd = [
        str(PLINK),
        "-batch",
        "-ssh",
        "-pw",
        PI_PASS,
        f"{PI_USER}@{PI_HOST}",
        remote,
    ]
    return run_cmd(cmd, timeout=timeout)


def parse_json_from_stdout(stdout: str) -> Dict:
    lines = [line.strip() for line in stdout.splitlines() if line.strip()]
    for line in reversed(lines):
        if line.startswith("{") and line.endswith("}"):
            return json.loads(line)
    raise ValueError(f"No JSON line found in stdout: {stdout[-1000:]}")


def parse_probe_final(stdout: str) -> Dict[str, int]:
    final = None
    for line in stdout.splitlines():
        if line.startswith("FINAL "):
            final = line.strip()
    if final is None:
        raise ValueError("No FINAL line found in probe output")
    pattern = (
        r"FINAL total=(?P<total>\d+)"
        r" note_on=(?P<note_on>\d+)"
        r" note_off=(?P<note_off>\d+)"
        r" cc=(?P<cc>\d+)"
        r" other=(?P<other>\d+)"
        r" participants_seen=(?P<participants_seen>\d+)"
        r" elapsed_ms=(?P<elapsed_ms>\d+)"
    )
    m = re.search(pattern, final)
    if not m:
        raise ValueError(f"Could not parse probe FINAL line: {final}")
    return {k: int(v) for k, v in m.groupdict().items()}


class OscReceiver:
    def __init__(self, port: int, duration_s: int):
        self.port = port
        self.duration_s = duration_s
        self.ready = threading.Event()
        self.done = threading.Event()
        self.error: Optional[str] = None
        self.counts = {
            "packets_total": 0,
            "note_packets": 0,
            "sustain_packets": 0,
            "other_packets": 0,
        }
        self.thread = threading.Thread(target=self._run, daemon=True)

    @staticmethod
    def _parse_addr(payload: bytes) -> str:
        nul = payload.find(b"\x00")
        if nul <= 0:
            return ""
        try:
            return payload[:nul].decode("utf-8", errors="ignore")
        except Exception:
            return ""

    def start(self):
        self.thread.start()
        if not self.ready.wait(timeout=5):
            raise RuntimeError("OSC receiver failed to become ready")

    def wait(self):
        if not self.done.wait(timeout=self.duration_s + 30):
            raise RuntimeError("OSC receiver timeout")
        if self.error:
            raise RuntimeError(self.error)
        return self.counts

    def _run(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4 * 1024 * 1024)
            sock.bind(("127.0.0.1", self.port))
            sock.settimeout(0.2)
            self.ready.set()
            end = time.time() + self.duration_s
            while time.time() < end:
                try:
                    data, _ = sock.recvfrom(2048)
                except socket.timeout:
                    continue
                except Exception as exc:
                    self.error = f"OSC recv error: {exc}"
                    return

                self.counts["packets_total"] += 1
                addr = self._parse_addr(data)
                if addr == "/avatar/parameters/sustain":
                    self.counts["sustain_packets"] += 1
                elif addr.startswith("/avatar/parameters/") and addr[len("/avatar/parameters/") :].isdigit():
                    self.counts["note_packets"] += 1
                else:
                    self.counts["other_packets"] += 1
        except Exception as exc:
            self.error = str(exc)
        finally:
            try:
                sock.close()
            except Exception:
                pass
            self.done.set()


def ensure_release_probe_built() -> None:
    exe = REPO_ROOT / ".cargo-target" / "release" / "rtp_probe.exe"
    if exe.exists():
        return
    cmd = ["cargo", "build", "--release", "--bin", "rtp_probe"]
    result = run_cmd(cmd, timeout=600000, cwd=REPO_ROOT / "src-tauri")
    if result.returncode != 0:
        raise RuntimeError(
            "Failed to build release rtp_probe:\nSTDOUT:\n"
            + result.stdout
            + "\nSTDERR:\n"
            + result.stderr
        )


def run_transport_probe_test(duration_s: int, sleep_s: float, probe_port: int) -> Dict:
    ensure_release_probe_built()
    probe_name = f"OSCMidiProbe2m_{int(time.time())}"
    probe_cmd = [
        str(REPO_ROOT / ".cargo-target" / "release" / "rtp_probe.exe"),
        "--name",
        probe_name,
        "--port",
        str(probe_port),
        "--duration",
        str(duration_s + 6),
        "--idle-timeout",
        "0",
    ]
    probe_proc = subprocess.Popen(
        probe_cmd,
        cwd=str(REPO_ROOT),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    time.sleep(2.0)

    pi_sender_script = f"""
import json, os, sys, time, mido
os.chdir('/home/Piano-LED-Visualizer')
sys.path.insert(0, '/home/Piano-LED-Visualizer')
from lib.midiports import MidiPorts

duration_s = {duration_s}
sleep_s = {sleep_s}
session_name = "{probe_name}"

ports = mido.get_output_names()
chosen = next((n for n in ports if session_name in n), None)
if not chosen:
    print(json.dumps({{"ok": False, "error": "probe session port not found", "ports": ports}}))
    raise SystemExit(2)

class DummySettings:
    def __init__(self, play_port):
        self._settings = {{
            "input_port": "default",
            "secondary_input_port": "default",
            "play_port": play_port,
        }}
    def get_setting_value(self, name):
        return self._settings.get(name, "default")
    def change_setting_value(self, name, value):
        self._settings[name] = value

mp = MidiPorts(DummySettings(chosen))
loops = 0
sustain_state = False
note_ok = 0
sustain_ok = 0
pitch_ok = 0
nonessential_calls = 0
send_false = 0
start = time.time()

while time.time() - start < duration_s:
    n = 48 + (loops % 36)
    sequence = [
        ("note", mido.Message("note_on", channel=0, note=n, velocity=100)),
        ("note", mido.Message("note_off", channel=0, note=n, velocity=0)),
    ]
    if loops % 10 == 0:
        sustain_state = not sustain_state
        sequence.append(("sustain", mido.Message("control_change", channel=0, control=64, value=(127 if sustain_state else 0))))
        bend = ((loops // 10) % 16384) - 8192
        sequence.append(("pitch", mido.Message("pitchwheel", channel=0, pitch=bend)))

    # Stress noise; expected to be filtered in RTP minimal mode.
    sequence.append(("nonessential", mido.Message("control_change", channel=0, control=1, value=(loops % 128))))
    sequence.append(("nonessential", mido.Message("clock")))

    for kind, msg in sequence:
        ok = bool(mp.send_to_playport(msg, source='2min_transport_probe'))
        if not ok:
            send_false += 1
            continue
        if kind == "note":
            note_ok += 1
        elif kind == "sustain":
            sustain_ok += 1
        elif kind == "pitch":
            pitch_ok += 1
        else:
            nonessential_calls += 1
    loops += 1
    if sleep_s > 0:
        time.sleep(sleep_s)

elapsed = time.time() - start
print(json.dumps({{
    "ok": True,
    "chosen_port": chosen,
    "duration_s": duration_s,
    "sleep_s": sleep_s,
    "loops": loops,
    "allowed_note_msgs": note_ok,
    "allowed_sustain_msgs": sustain_ok,
    "allowed_pitch_msgs": pitch_ok,
    "allowed_total": note_ok + sustain_ok + pitch_ok,
    "filtered_or_nonessential_calls": nonessential_calls,
    "send_false": send_false,
    "actual_play_port": getattr(mp, 'actual_play_port', None),
    "rtp_active": bool(mp.is_rtp_session_active()),
    "elapsed_s": elapsed
}}))
"""

    sender = run_pi_python(pi_sender_script, timeout=duration_s + 120)

    try:
        probe_out, probe_err = probe_proc.communicate(timeout=duration_s + 60)
    except subprocess.TimeoutExpired:
        probe_proc.kill()
        probe_out, probe_err = probe_proc.communicate()
        raise RuntimeError("rtp_probe timeout")

    if sender.returncode != 0:
        raise RuntimeError(
            f"Pi transport sender failed rc={sender.returncode}\nSTDOUT:\n{sender.stdout}\nSTDERR:\n{sender.stderr}"
        )
    if probe_proc.returncode != 0:
        raise RuntimeError(
            f"rtp_probe failed rc={probe_proc.returncode}\nSTDOUT:\n{probe_out}\nSTDERR:\n{probe_err}"
        )

    sender_json = parse_json_from_stdout(sender.stdout)
    probe_final = parse_probe_final(probe_out)

    expected_probe_total = sender_json["allowed_total"]
    received_probe_total = probe_final["total"]
    return {
        "sender": sender_json,
        "probe_stdout": probe_out,
        "probe_stderr": probe_err,
        "probe_final": probe_final,
        "comparison": {
            "expected_probe_total": expected_probe_total,
            "received_probe_total": received_probe_total,
            "loss_transport_total": expected_probe_total - received_probe_total,
            "expected_note_on": sender_json["allowed_note_msgs"] // 2,
            "received_note_on": probe_final["note_on"],
            "loss_note_on": (sender_json["allowed_note_msgs"] // 2) - probe_final["note_on"],
            "expected_note_off": sender_json["allowed_note_msgs"] // 2,
            "received_note_off": probe_final["note_off"],
            "loss_note_off": (sender_json["allowed_note_msgs"] // 2) - probe_final["note_off"],
            "expected_cc_total": sender_json["allowed_sustain_msgs"],
            "received_cc_total": probe_final["cc"],
            "loss_cc_total": sender_json["allowed_sustain_msgs"] - probe_final["cc"],
            "expected_other_total": sender_json["allowed_pitch_msgs"],
            "received_other_total": probe_final["other"],
            "loss_other_total": sender_json["allowed_pitch_msgs"] - probe_final["other"],
        },
    }


def run_e2e_osc_test(duration_s: int, sleep_s: float, osc_port: int) -> Dict:
    receiver = OscReceiver(port=osc_port, duration_s=duration_s + 5)
    receiver.start()
    time.sleep(0.5)

    pi_sender_script = f"""
import json, os, sys, time, mido
os.chdir('/home/Piano-LED-Visualizer')
sys.path.insert(0, '/home/Piano-LED-Visualizer')
from lib.midiports import MidiPorts
from lib.usersettings import UserSettings

duration_s = {duration_s}
sleep_s = {sleep_s}

mp = MidiPorts(UserSettings())
loops = 0
sustain_state = False
note_ok = 0
sustain_ok = 0
pitch_ok = 0
nonessential_calls = 0
send_false = 0
start = time.time()

while time.time() - start < duration_s:
    n = 48 + (loops % 36)
    sequence = [
        ("note", mido.Message("note_on", channel=0, note=n, velocity=100)),
        ("note", mido.Message("note_off", channel=0, note=n, velocity=0)),
    ]
    if loops % 10 == 0:
        sustain_state = not sustain_state
        sequence.append(("sustain", mido.Message("control_change", channel=0, control=64, value=(127 if sustain_state else 0))))
        bend = ((loops // 10) % 16384) - 8192
        sequence.append(("pitch", mido.Message("pitchwheel", channel=0, pitch=bend)))

    # Intentional stress noise; should be filtered in RTP minimal mode.
    sequence.append(("nonessential", mido.Message("control_change", channel=0, control=1, value=(loops % 128))))
    sequence.append(("nonessential", mido.Message("clock")))

    for kind, msg in sequence:
        ok = bool(mp.send_to_playport(msg, source='2min_intensive'))
        if not ok:
            send_false += 1
            continue
        if kind == "note":
            note_ok += 1
        elif kind == "sustain":
            sustain_ok += 1
        elif kind == "pitch":
            pitch_ok += 1
        else:
            nonessential_calls += 1
    loops += 1
    if sleep_s > 0:
        time.sleep(sleep_s)

elapsed = time.time() - start
print(json.dumps({{
    "ok": True,
    "duration_s": duration_s,
    "sleep_s": sleep_s,
    "loops": loops,
    "allowed_note_msgs": note_ok,
    "allowed_sustain_msgs": sustain_ok,
    "allowed_pitch_msgs": pitch_ok,
    "allowed_total": note_ok + sustain_ok + pitch_ok,
    "expected_osc_msgs": note_ok + sustain_ok,
    "filtered_or_nonessential_calls": nonessential_calls,
    "send_false": send_false,
    "actual_play_port": getattr(mp, 'actual_play_port', None),
    "rtp_active": bool(mp.is_rtp_session_active()),
    "elapsed_s": elapsed
}}))
"""

    sender = run_pi_python(pi_sender_script, timeout=duration_s + 120)
    osc_counts = receiver.wait()

    if sender.returncode != 0:
        raise RuntimeError(
            f"Pi e2e sender failed rc={sender.returncode}\nSTDOUT:\n{sender.stdout}\nSTDERR:\n{sender.stderr}"
        )

    sender_json = parse_json_from_stdout(sender.stdout)
    return {
        "sender": sender_json,
        "sender_stdout": sender.stdout,
        "sender_stderr": sender.stderr,
        "osc_receiver": osc_counts,
        "comparison": {
            "expected_note": sender_json["allowed_note_msgs"],
            "osc_note": osc_counts["note_packets"],
            "loss_sender_to_osc_note": sender_json["allowed_note_msgs"] - osc_counts["note_packets"],
            "expected_sustain": sender_json["allowed_sustain_msgs"],
            "osc_sustain": osc_counts["sustain_packets"],
            "loss_sender_to_osc_sustain": sender_json["allowed_sustain_msgs"] - osc_counts["sustain_packets"],
            "expected_pitch": sender_json["allowed_pitch_msgs"],
            "expected_osc_total": sender_json["expected_osc_msgs"],
            "osc_total": osc_counts["note_packets"] + osc_counts["sustain_packets"],
            "loss_sender_to_osc_total": sender_json["expected_osc_msgs"]
            - (osc_counts["note_packets"] + osc_counts["sustain_packets"]),
        },
    }


def main():
    parser = argparse.ArgumentParser(description="2-minute RTP stability test (Pi transport + OSC e2e)")
    parser.add_argument("--duration", type=int, default=120)
    parser.add_argument("--sleep", type=float, default=0.0015)
    parser.add_argument("--probe-port", type=int, default=5012)
    parser.add_argument("--osc-port", type=int, default=9000)
    parser.add_argument("--skip-transport", action="store_true")
    parser.add_argument("--skip-e2e", action="store_true")
    args = parser.parse_args()

    report = {
        "timestamp": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "duration_s": args.duration,
        "sleep_s": args.sleep,
        "probe_port": args.probe_port,
        "osc_port": args.osc_port,
        "release_app_expected": str(REPO_ROOT / ".cargo-target" / "release" / "OSCMidi.exe"),
    }

    if not args.skip_transport:
        report["transport_probe"] = run_transport_probe_test(
            duration_s=args.duration,
            sleep_s=args.sleep,
            probe_port=args.probe_port,
        )

    if not args.skip_e2e:
        report["e2e_osc"] = run_e2e_osc_test(
            duration_s=args.duration,
            sleep_s=args.sleep,
            osc_port=args.osc_port,
        )

    out_dir = REPO_ROOT / "reports"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"stability_2min_intensive_{now_stamp()}.json"
    out_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({"ok": True, "report_path": str(out_path)}, indent=2))


if __name__ == "__main__":
    main()
