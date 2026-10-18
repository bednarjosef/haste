#!/usr/bin/env bash
# The attack indicator render smoke (review item: the crosshair recharge bar): a scripted
# attack at the sky, screenshots of the bar under the crosshair as it fills, and the HUD
# frame debug log as textual evidence (the "attack N" sprite count per frame).
# usage: scripts/checks/attack-indicator.sh
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=${HASTE_CHECK_NAME:-AtkInd$$}
NAME=${NAME:0:16}
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
LOG="$ROOT/target/feature-checks/attack-indicator.log"
mkdir -p "$(dirname "$LOG")"

timeout 120 env RUST_LOG=haste=debug "$BIN" --server "127.0.0.1:$PORT" --username "$NAME" \
  --script "$ROOT/scripts/checks/attack-indicator.txt" --seconds 90 >"$LOG" 2>&1
r=$?

echo "== client exit $r, log $LOG"
echo "== screenshots (target/shots/script-indicator-*.png)"
ls -la "$ROOT/target/shots"/script-indicator-*.png 2>/dev/null || echo "none"
echo "== attack indicator sprites per frame around the swing (debug log)"
grep "hud frame" "$LOG" | grep -v "attack 0" | tail -30
exit $r
