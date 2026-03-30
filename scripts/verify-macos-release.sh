#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: verify-macos-release.sh <dmg-path> [--skip-launch]" >&2
  exit 1
fi

DMG_PATH="$1"
SKIP_LAUNCH="false"
if [[ "${2:-}" == "--skip-launch" ]]; then
  SKIP_LAUNCH="true"
fi

if [[ ! -f "$DMG_PATH" ]]; then
  echo "DMG not found: $DMG_PATH" >&2
  exit 1
fi

echo "Verifying DMG gatekeeper status: $DMG_PATH"
DMG_CHECK="$(spctl --assess --type open --context context:primary-signature -vv "$DMG_PATH" 2>&1 || true)"
echo "$DMG_CHECK"
if [[ "$DMG_CHECK" != *"accepted"* ]]; then
  echo "DMG failed Gatekeeper verification." >&2
  exit 1
fi

MOUNT_PLIST="$(mktemp)"
hdiutil attach "$DMG_PATH" -nobrowse -readonly -plist >"$MOUNT_PLIST"
VOLUME_PATH="$(python3 - "$MOUNT_PLIST" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as fh:
    data = plistlib.load(fh)

entities = data.get("system-entities", [])
mounts = [item.get("mount-point") for item in entities if item.get("mount-point")]
if not mounts:
    raise SystemExit(1)

print(mounts[-1])
PY
)"
APP_PATH="$(find "$VOLUME_PATH" -maxdepth 1 -name '*.app' -type d | head -1)"

cleanup() {
  rm -f "${MOUNT_PLIST:-}" >/dev/null 2>&1 || true
  if [[ -n "${VOLUME_PATH:-}" ]] && [[ -d "$VOLUME_PATH" ]]; then
    hdiutil detach "$VOLUME_PATH" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [[ -z "$APP_PATH" ]]; then
  echo "No .app found inside mounted DMG." >&2
  exit 1
fi

echo "Verifying app gatekeeper status: $APP_PATH"
APP_CHECK="$(spctl -a -vv "$APP_PATH" 2>&1 || true)"
echo "$APP_CHECK"
if [[ "$APP_CHECK" != *"accepted"* ]]; then
  echo "App failed Gatekeeper verification." >&2
  exit 1
fi

if [[ "$SKIP_LAUNCH" == "true" ]]; then
  exit 0
fi

APP_BIN="$APP_PATH/Contents/MacOS/PortPilot"
if [[ ! -x "$APP_BIN" ]]; then
  echo "App binary not found: $APP_BIN" >&2
  exit 1
fi

echo "Launching mounted app binary for smoke verification"
"$APP_BIN" >/tmp/portpilot-release-smoke.log 2>&1 &
APP_PID=$!
sleep 5

if ! kill -0 "$APP_PID" >/dev/null 2>&1; then
  echo "App binary exited unexpectedly during smoke verification." >&2
  cat /tmp/portpilot-release-smoke.log >&2 || true
  exit 1
fi

kill "$APP_PID" >/dev/null 2>&1 || true
wait "$APP_PID" 2>/dev/null || true
