#!/usr/bin/env bash
# One-command join smoke test: builds release haste, starts a private test server, joins it
# headless for 20 s, then greps both logs for the server's verdict on our client and for
# client panics. Prints PASS or FAIL, exit 0/1. Always stops the server it started.
# usage: scripts/smoke.sh [port]   (default 25618; server copy $HASTE_CACHE/servers/smoke)
#
# Three server-log lines appear on every healthy join and do not fail the run (each is
# still printed, marked known-benign):
#   "lost connection: Disconnected"  haste's own goodbye after its 20 s; a real disconnect
#                                    still fails, via haste's non-zero exit and the reason
#                                    text differing from a bare "Disconnected"
#   "moved too quickly! X,Y,Z" where X,Y,Z is the login position: the spawn position ack
#                                    (the server compares it against the pre-login spot)
#   "No key layers in MapLike[{}]"   flat-world preset datapack noise at server start,
#                                    logged before the client even connects
set -uo pipefail
PORT=${1:-25618}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
LOG="$CACHE/servers/smoke/logs/latest.log"
cd "$ROOT" || exit 1
mkdir -p target/smoke

cleanup() {
  local pid children
  if [ -n "${SERVER_PID:-}" ]; then
    children=$(pgrep -P "$SERVER_PID" 2>/dev/null || true)  # timeout(1)'s java child
    kill "$SERVER_PID" 2>/dev/null || true                  # timeout forwards TERM to java
    for pid in $children; do kill "$pid" 2>/dev/null || true; done
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  # A stray java from our copy only: matched by working directory, never another server.
  for pid in $(pgrep -f fabric-server-launch 2>/dev/null || true); do
    if [ "$(readlink "/proc/$pid/cwd" 2>/dev/null || true)" = "$CACHE/servers/smoke" ]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT

echo "== building release haste (first build takes a few minutes)"
cargo build --release -q --bin haste || { echo "FAIL: cargo build"; exit 1; }

echo "== starting private test server on port $PORT"
[ -f "$LOG" ] && : > "$LOG"  # truncate: an old "Done (" must not count as this run's
"$ROOT/scripts/test-server.sh" smoke "$PORT" 180 > target/smoke/server.out 2>&1 &
SERVER_PID=$!

echo -n "== waiting up to 120 s for the server "
up=
for _ in $(seq 1 240); do
  grep -q 'Done (' "$LOG" 2>/dev/null && { up=1; break; }
  kill -0 "$SERVER_PID" 2>/dev/null || break
  sleep 0.5
done
if [ -z "$up" ]; then
  echo
  echo "FAIL: server did not start (or died); last lines of target/smoke/server.out:"
  tail -5 target/smoke/server.out 2>/dev/null
  exit 1
fi
echo up

echo "== running headless haste for 20 s (target/smoke/haste.log)"
target/release/haste --headless --server "127.0.0.1:${SMOKE_CLIENT_PORT:-$PORT}" \
  --username smoke --seconds 20 > target/smoke/haste.log 2>&1
RC=$?
echo "-- haste exit code $RC; last 5 lines of its log:"
tail -5 target/smoke/haste.log

FAIL=
[ "$RC" -eq 0 ] || FAIL="haste exited $RC"

# The login position: a "moved too quickly" carrying exactly it is the spawn ack, not a move.
LOGIN_POS=$(sed -n 's/.*logged in with entity id [0-9]* at (\([^)]*\)).*/\1/p' "$LOG" 2>/dev/null | head -1 | tr -d ' ')
SPAWN_ACK='__no_login_position__'
[ -n "$LOGIN_POS" ] && SPAWN_ACK="*moved too quickly! $LOGIN_POS*"

for pat in 'kicked' 'lost connection' 'moved wrongly' 'moved too quickly' 'Exception' 'ERROR'; do
  while IFS= read -r line; do
    benign=
    case "$line" in
      *"lost connection: Disconnected") benign="haste's goodbye after its 20 s";;
      *"No key layers in MapLike[{}]") benign="flat-preset datapack noise before login";;
      $SPAWN_ACK) benign="spawn position ack (equals the login position)";;
    esac
    echo "server: $line"
    if [ -n "$benign" ]; then
      echo "        known-benign: $benign"
    else
      FAIL="${FAIL:+$FAIL; }server: $line"
    fi
  done < <(grep -i -- "$pat" "$LOG" 2>/dev/null || true)
done

for pat in 'panicked' 'Error:'; do
  while IFS= read -r line; do
    echo "client: $line"
    FAIL="${FAIL:+$FAIL; }client: $line"
  done < <(grep -F -- "$pat" target/smoke/haste.log 2>/dev/null || true)
done

if [ -n "$FAIL" ]; then
  echo "FAIL: $FAIL"
  exit 1
fi
echo PASS
