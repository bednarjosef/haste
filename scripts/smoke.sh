#!/usr/bin/env bash
# One-command join smoke test: builds haste, starts a private test server, joins it headless
# for 20 s and greps both logs for the server's verdict (kicks, "moved wrongly") or client
# panics. Prints PASS or FAIL (exit 0/1).
# usage: scripts/smoke.sh [port]   (default 25618)
set -uo pipefail
PORT=${1:-25618}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
cd "$ROOT"
LOG="$CACHE/servers/smoke/logs/latest.log"
mkdir -p target/smoke

cleanup() {
  if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null
    for _ in $(seq 1 20); do kill -0 "$SERVER_PID" 2>/dev/null || break; sleep 0.5; done
    kill -9 "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  # exec'd timeout/java may not be gone with the script's own PID; catch stragglers.
  local pid
  for pid in $(pgrep -f "servers/smoke.*fabric-server-launch" || true); do
    [ "$pid" != "$$" ] && kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

echo "== building release haste"
cargo build --release -q --bin haste || { echo "FAIL: cargo build"; exit 1; }

echo "== starting test server on port $PORT"
# remember the old log's line count so a stale "Done (" from a previous run doesn't count
OLD_LINES=0
[ -f "$LOG" ] && OLD_LINES=$(wc -l < "$LOG")
"$ROOT/scripts/test-server.sh" smoke "$PORT" 180 &
SERVER_PID=$!

echo -n "== waiting for server startup "
up=0
for _ in $(seq 1 240); do
  if [ -f "$LOG" ] && [ "$(wc -l < "$LOG")" -gt "$OLD_LINES" ] && \
     tail -n "+$((OLD_LINES + 1))" "$LOG" | grep -q 'Done ('; then
    up=1; break
  fi
  sleep 0.5
done
[ "$up" = 1 ] || { echo; echo "FAIL: server did not start"; tail -5 "$LOG" 2>/dev/null; exit 1; }
echo "up"

echo "== running headless haste"
target/release/haste --headless --server "127.0.0.1:$PORT" --username smoke --seconds 20 \
  > target/smoke/haste.log 2>&1
HASTE_RC=$?
echo "-- last 5 lines of target/smoke/haste.log:"
tail -5 target/smoke/haste.log

FAIL=""
[ "$HASTE_RC" -eq 0 ] || FAIL="haste exited $HASTE_RC"
NEW_LOG=$(tail -n "+$((OLD_LINES + 1))" "$LOG")
for pat in 'kicked' 'lost connection' 'moved wrongly' 'moved too quickly' 'Exception' 'ERROR'; do
  hits=$(printf '%s\n' "$NEW_LOG" | grep -i "$pat" || true)
  [ -n "$hits" ] && { echo "-- server log matches '$pat':"; echo "$hits"; FAIL="${FAIL:+$FAIL; }server log: $pat"; }
done
for pat in 'panicked' 'Error:'; do
  hits=$(grep -E "$pat" target/smoke/haste.log || true)
  [ -n "$hits" ] && { echo "-- client log matches '$pat':"; echo "$hits"; FAIL="${FAIL:+$FAIL; }client log: $pat"; }
done

if [ -n "$FAIL" ]; then echo "FAIL: $FAIL"; exit 1; fi
echo PASS
