#!/usr/bin/env python3
"""Real-HTTP regression rig for cleanup ownership, persistence, and lock latency."""

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import urllib.request


BINARY = pathlib.Path(sys.argv[1]).resolve()


def executable(path: pathlib.Path, body: str) -> None:
    path.write_text("#!/bin/bash\n" + body)
    path.chmod(0o755)


def run_case(mode: str) -> dict:
    root = pathlib.Path(tempfile.mkdtemp(prefix=f"emumi-{mode}-"))
    home, state, sdk, bindir = [root / name for name in ("home", "state", "sdk", "bin")]
    for path in (
        home / ".config/emumi",
        home / ".android/avd/Device_4.avd",
        state,
        sdk / "emulator",
        sdk / "platform-tools",
        sdk / "cmdline-tools/latest/bin",
        bindir,
        root / "tmp",
    ):
        path.mkdir(parents=True, exist_ok=True)
    executable(sdk / "emulator/emulator", 'printf "Device_4\\n"\n')
    if mode in {"legacy_hold", "adopt", "restart"}:
        executable(
            sdk / "platform-tools/adb",
            '''case "$*" in
devices) printf 'List of devices attached\\nemulator-5554\\tdevice\\n';;
*'emu avd name'*) printf 'Device_4\\nOK\\n';;
*sys.boot_completed*) printf '1\\n';;
esac
''',
        )
    else:
        executable(sdk / "platform-tools/adb", "exit 0\n")
    executable(sdk / "cmdline-tools/latest/bin/avdmanager", "exit 0\n")
    executable(bindir / "systemd-run", "exit 0\n")
    executable(bindir / "prlimit", "exit 0\n")
    executable(
        bindir / "systemctl",
        '''printf '%s\\n' "$*" >> "$WIRE_RIG/calls"
case "$*" in
  *list-units*) printf 'emumi-Device_4-123.scope loaded active running\\n';;
  *is-active*) exit 0;;
  *stop*) exit 1;;
esac
''',
    )
    phase = {"legacy_hold": "thermal_hold", "restart": "booting"}.get(mode, "ready")
    config = {
        "android_sdk_path": str(sdk),
        "policy_version": 2,
        "profile_options": {"Device_4": {"adb_port": 5554}},
        "desired_running_profiles": ["Device_4"],
        "lifecycle_states": {
            "Device_4": {
                "status": {
                    "phase": phase,
                    "desired_running": True,
                    "generation": 1,
                    "stop_requested": False,
                    "consecutive_failures": 0,
                    "next_retry_at": int(time.time()) + 30 if mode == "legacy_hold" else None,
                    "last_failure": "Thermal stop at 95 C" if mode == "legacy_hold" else None,
                    "scope": None if mode == "adopt" else "emumi-Device_4-123.scope",
                    "pid": None if mode == "adopt" else 999999,
                    "cpu_quota_percent": 80,
                    "cpu_temperature_c": 95.0 if mode == "legacy_hold" else 60.0,
                    "snapshot_state": None,
                }
            }
        },
    }
    config_path = home / ".config/emumi/config.json"
    config_path.write_text(json.dumps(config))
    env = os.environ | {
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_STATE_HOME": str(state),
        "TMPDIR": str(root / "tmp"),
        "PATH": f"{bindir}:/usr/bin:/bin",
        "WIRE_RIG": str(root),
        "EMUMI_SERVER_ONLY": "1",
        "ANDROID_HOME": str(sdk),
        "ANDROID_SDK_ROOT": str(sdk),
    }
    with (root / "manager.log").open("w") as output:
        process = subprocess.Popen([BINARY], env=env, stdout=output, stderr=output)
    try:
        endpoint = state / "emumi/api-url"
        for _ in range(100):
            if endpoint.exists():
                break
            time.sleep(0.02)
        url = endpoint.read_text().strip()

        def request(path: str, post: bool = False) -> dict:
            req = urllib.request.Request(url + path, method="POST" if post else "GET")
            with urllib.request.urlopen(req, timeout=2) as response:
                return json.load(response)

        if mode == "mutex":
            executable(bindir / "which", '/bin/sleep 0.6\nexec /usr/bin/which "$@"\n')
            started = time.monotonic()
            request("/api/profiles/Device_4/stop", True)
            elapsed = time.monotonic() - started
            assert elapsed < 0.5, f"stop intent took {elapsed:.3f}s"
            return {"stop_seconds": elapsed}

        if mode == "persistence":
            current = {}
            for _ in range(80):
                current = request("/api/state")["device_supervision"]["Device_4"]
                if current["phase"] == "backoff":
                    break
                time.sleep(0.2)
            saved = json.loads(config_path.read_text())["lifecycle_states"]["Device_4"]["status"]
            assert current["phase"] == "backoff"
            assert saved["phase"] == "backoff"
            assert saved["next_retry_at"] == current["next_retry_at"]
            return {"phase": saved["phase"], "retry": saved["next_retry_at"]}

        if mode == "legacy_hold":
            time.sleep(0.5)
            status = request("/api/state")["device_supervision"]["Device_4"]
            assert status["phase"] != "thermal_hold"
            assert "cpu_quota_percent" not in status
            assert "cpu_temperature_c" not in status
            return {"phase": status["phase"]}

        if mode == "restart":
            time.sleep(32)
            status = request("/api/state")["device_supervision"]["Device_4"]
            readiness = request("/api/ports/5554/readiness")
            assert status["phase"] == "ready"
            assert readiness["ready"] is True
            return {"phase": status["phase"], "ready": readiness["ready"]}

        if mode == "adopt":
            time.sleep(0.5)
            request("/api/profiles/Device_4/stop", True)
            time.sleep(0.5)
            status = request("/api/state")["device_supervision"]["Device_4"]
            calls = (root / "calls").read_text()
            assert "--user stop emumi-Device_4-123.scope" in calls
            assert status["scope"] == "emumi-Device_4-123.scope"
            return {"phase": status["phase"], "ownership_retained": True}

        request("/api/profiles/Device_4/stop", True)
        time.sleep(0.7)
        status = request("/api/state")["device_supervision"]["Device_4"]
        calls = (root / "calls").read_text()
        assert "--user stop emumi-Device_4-123.scope" in calls
        assert status["phase"] == "stopping"
        assert status["scope"] == "emumi-Device_4-123.scope"
        assert status["pid"] == 999999
        assert "could not stop" in status["last_failure"].lower()
        return {"phase": status["phase"], "ownership_retained": True}
    finally:
        process.terminate()
        process.wait(timeout=5)


results = {
    mode: run_case(mode)
    for mode in ("cleanup", "persistence", "mutex", "legacy_hold", "adopt", "restart")
}
print("PASS manager safety wire " + json.dumps(results, sort_keys=True))
