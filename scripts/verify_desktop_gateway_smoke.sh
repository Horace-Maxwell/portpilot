#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_PATH="${1:-$ROOT_DIR/src-tauri/target/debug/bundle/macos/PortPilot.app}"
PORT_START="${PORT_START:-42300}"
PORT_END="${PORT_END:-42325}"
KEEP_OPEN="${KEEP_OPEN:-0}"
REQUIRE_TRUSTED="${REQUIRE_TRUSTED:-0}"
APP_NAME="PortPilot"

if [[ ! -d "$APP_PATH" ]]; then
  echo "App bundle not found: $APP_PATH" >&2
  exit 1
fi

cleanup() {
  if [[ "$KEEP_OPEN" != "1" ]]; then
    osascript -e "tell application \"$APP_NAME\" to quit" >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

open -na "$APP_PATH"

HTTP_PORT=""
HTTPS_PORT=""
for _ in $(seq 1 30); do
  for port in $(seq "$PORT_START" "$PORT_END"); do
    if [[ -z "$HTTP_PORT" ]] && curl -sS --max-time 1 -I "http://127.0.0.1:$port/" >/dev/null 2>&1; then
      HTTP_PORT="$port"
    fi
    if [[ -z "$HTTPS_PORT" ]] && curl -k -sS --max-time 1 -I "https://127.0.0.1:$port/" >/dev/null 2>&1; then
      HTTPS_PORT="$port"
    fi
  done

  if [[ -n "$HTTP_PORT" && -n "$HTTPS_PORT" ]]; then
    break
  fi
  sleep 1
done

if [[ -z "$HTTP_PORT" || -z "$HTTPS_PORT" ]]; then
  echo "Gateway smoke failed. HTTP port: ${HTTP_PORT:-missing}, HTTPS port: ${HTTPS_PORT:-missing}" >&2
  exit 1
fi

if [[ "$REQUIRE_TRUSTED" == "1" ]]; then
  if ! curl -sS --max-time 2 -I "https://127.0.0.1:$HTTPS_PORT/" >/dev/null 2>&1; then
    echo "HTTPS listener is reachable, but the certificate is not trusted yet on https://127.0.0.1:$HTTPS_PORT/." >&2
    exit 1
  fi
fi

echo "HTTP gateway:  http://127.0.0.1:$HTTP_PORT/"
echo "HTTPS gateway: https://127.0.0.1:$HTTPS_PORT/"
echo "Desktop smoke passed."
