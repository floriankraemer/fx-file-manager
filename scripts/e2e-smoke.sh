#!/bin/sh
# Launch fx-gui on a virtual X server, check it stays up, and capture the
# screen. A crash or an early exit fails the script; the screenshot is for
# a human to inspect, since pixels are not asserted.
set -eu

bin=${1:-target/release/fx-gui}
dir=${2:-/usr}
out=${3:-dist/e2e/screenshot.png}

mkdir -p "$(dirname "$out")"

xvfb-run -a -s "-screen 0 1600x1200x24" sh -c '
  "$0" "$1" &
  pid=$!
  sleep 4
  kill -0 "$pid" 2>/dev/null || { echo "fx-gui exited early" >&2; exit 1; }
  import -window root "$2"
  kill "$pid"
  wait "$pid" 2>/dev/null || true
  echo "smoke ok: screenshot at $2"
' "$bin" "$dir" "$out"
