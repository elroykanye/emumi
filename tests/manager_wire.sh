#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
binary="$repo_root/target/debug/emumi"
rig=$(mktemp -d)
manager_pid=""

cleanup() {
  if [[ -n "$manager_pid" ]] && kill -0 "$manager_pid" 2>/dev/null; then
    kill "$manager_pid" 2>/dev/null || true
    wait "$manager_pid" 2>/dev/null || true
  fi
  rm -rf -- "$rig"
}
trap cleanup EXIT INT TERM

mkdir -p "$rig/home/.config/emumi" "$rig/state" "$rig/sdk/emulator" \
  "$rig/sdk/platform-tools" "$rig/sdk/cmdline-tools/latest/bin" "$rig/bin"

printf '%s\n' '{"android_sdk_path":"'"$rig"'/sdk","jdk_path":"","profile_options":{},"desired_running_profiles":[]}' \
  >"$rig/home/.config/emumi/config.json"

for tool in "$rig/sdk/emulator/emulator" "$rig/sdk/platform-tools/adb"; do
  printf '%s\n' '#!/usr/bin/env bash' 'sleep 5' >"$tool"
  chmod +x "$tool"
done
for tool in avdmanager systemd-run systemctl prlimit; do
  path="$rig/bin/$tool"
  if [[ "$tool" == avdmanager ]]; then
    path="$rig/sdk/cmdline-tools/latest/bin/$tool"
  fi
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$path"
  chmod +x "$path"
done

start_manager() {
  env HOME="$rig/home" XDG_CONFIG_HOME="$rig/home/.config" XDG_STATE_HOME="$rig/state" \
    PATH="$rig/bin:/usr/bin:/bin" EMUMI_SERVER_ONLY=1 "$binary" \
    >"$rig/manager.out" 2>"$rig/manager.err" &
  manager_pid=$!
}

wait_for_endpoint() {
  for _ in $(seq 1 100); do
    if [[ -s "$rig/state/emumi/api-url" ]]; then
      return 0
    fi
    sleep 0.02
  done
  echo "manager endpoint was not published" >&2
  sed -n '1,120p' "$rig/manager.err" >&2 || true
  return 1
}

start_manager
wait_for_endpoint
first_url=$(tr -d '\n' <"$rig/state/emumi/api-url")
curl --silent --show-error --max-time 1 --write-out '\n%{time_total}' "$first_url/api/state" \
  >"$rig/state-response"
first_time=$(tail -n 1 "$rig/state-response")
head -n -1 "$rig/state-response" >"$rig/state.json"
grep -q '"manager_id"' "$rig/state.json"
grep -q '"device_supervision"' "$rig/state.json"
for forbidden_field in '"stats"' '"resource_warnings"' '"device_stats"'; do
  if grep -q "$forbidden_field" "$rig/state.json"; then
    echo "host telemetry leaked into /api/state: $forbidden_field" >&2
    exit 1
  fi
done
awk -v elapsed="$first_time" 'BEGIN { if (elapsed >= 0.5) exit 1 }'

curl --silent --show-error --max-time 1 --request POST --write-out '\n%{time_total}' \
  "$first_url/api/profiles/Device_4/stop" >"$rig/stop-response"
stop_time=$(tail -n 1 "$rig/stop-response")
head -n -1 "$rig/stop-response" | grep -q 'stop requested'
awk -v elapsed="$stop_time" 'BEGIN { if (elapsed >= 0.5) exit 1 }'

set +e
timeout 2 env HOME="$rig/home" XDG_CONFIG_HOME="$rig/home/.config" XDG_STATE_HOME="$rig/state" \
  PATH="$rig/bin:/usr/bin:/bin" EMUMI_SERVER_ONLY=1 "$binary" \
  >"$rig/duplicate.out" 2>"$rig/duplicate.err"
duplicate_exit=$?
set -e
[[ "$duplicate_exit" -eq 1 ]]
grep -q 'already running' "$rig/duplicate.err"

kill "$manager_pid"
wait "$manager_pid" 2>/dev/null || true
manager_pid=""
stale_url=$(tr -d '\n' <"$rig/state/emumi/api-url")
[[ "$stale_url" == "$first_url" ]]

start_manager
for _ in $(seq 1 100); do
  second_url=$(tr -d '\n' <"$rig/state/emumi/api-url" 2>/dev/null || true)
  if [[ -n "$second_url" ]] && curl --silent --fail --max-time 1 "$second_url/api/state" \
    >"$rig/second-state.json"; then
    break
  fi
  sleep 0.02
done
grep -q '"manager_id"' "$rig/second-state.json"

echo "PASS manager wire: exclusive lock, atomic stale endpoint replacement, cached state ${first_time}s, stop intent ${stop_time}s"
python3 "$repo_root/tests/manager_safety_wire.py" "$binary"
