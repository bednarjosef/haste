#!/usr/bin/env bash
# Boat steering live check (scripts/checks/boat-steering.txt) against the running test
# server on 127.0.0.1:25691. It builds a still water lane in this branch's own area
# (x 370..394, z -2..46, far from the shared spawn floor), summons an oak boat, lets the
# scripted client mount it (use-item on the entity) and paddle it forward (W, 3 s) and
# turn left (A, 1 s), then reads the server's own verdict back over RCON: the boat's Pos
# before and after, that it carried the player, and that the server log holds no
# "moved wrongly"/"moved too quickly" for the boat. No server is started, and nothing
# outside this branch's area is touched.
# usage: scripts/checks/boat-steering.sh
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=BoatCheck
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
SCRIPT="$ROOT/scripts/checks/boat-steering.txt"
LOG="$ROOT/target/boat-steering.log"
cd "$ROOT" || exit 1
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }

# The running server's own log: the java process whose working directory holds a
# server.properties naming our port (the server may use a different HASTE_CACHE).
SERVER_LOG=
for p in /proc/[0-9]*; do
  cwd=$(readlink "$p/cwd" 2>/dev/null) || continue
  if [ -f "$cwd/server.properties" ] && grep -q "^server-port=$PORT\$" "$cwd/server.properties" 2>/dev/null; then
    SERVER_LOG="$cwd/logs/latest.log"
  fi
done
echo "== server log: ${SERVER_LOG:-<not found for port $PORT>}"
SERVER_LINES=0
[ -n "$SERVER_LOG" ] && SERVER_LINES=$(wc -l <"$SERVER_LOG" 2>/dev/null || echo 0)

: >"$LOG"
echo "== joining headless as $NAME with $SCRIPT"
"$BIN" --server "127.0.0.1:$PORT" --username "$NAME" --script "$SCRIPT" --seconds 90 >"$LOG" 2>&1 &
CLIENT_PID=$!

up=
for _ in $(seq 1 120); do
  rcon list 2>/dev/null | grep -q "$NAME" && { up=1; break; }
  sleep 0.5
done
if [ -z "$up" ]; then
  echo "FAIL: $NAME never joined (see $LOG)"
  wait "$CLIENT_PID" 2>/dev/null
  exit 1
fi

# The RCON setup must land inside the script's first `wait 400` (20 s): the teleport into
# the area (so the server loads its chunks — chunks without a player in them reject fills,
# so every fill runs `execute at` the player), then a clean rebuild of the rig (y 70..74
# cleared, the floor, the one-layer water lane) and the boat summon.
rcon "tp $NAME 380.5 71 24" >/dev/null
sleep 4
rcon "execute at $NAME run fill 370 70 -2 394 74 46 air" >/dev/null
rcon "execute at $NAME run fill 370 69 -2 394 69 46 stone" >/dev/null
rcon "execute at $NAME run fill 376 70 6 386 70 42 water" >/dev/null
rcon "kill @e[type=oak_boat]" >/dev/null
rcon "summon minecraft:oak_boat 380.5 70.9 8.5" >/dev/null
sleep 3
echo "== mounting spot"
rcon "tp $NAME 380.5 70 6.0 0 30" 2>&1 | tail -1
SPOT=$(rcon "data get entity $NAME Pos" 2>/dev/null)
echo "player spot: $SPOT"
BOAT_REST=$(rcon "data get entity @e[type=oak_boat,limit=1,sort=nearest] Pos" 2>/dev/null)
echo "boat rest: $BOAT_REST"

# The script's `click right` steps mount the boat, then `wait 120` holds it at rest.
sleep 9
MOUNTED=$(rcon "data get entity @e[type=oak_boat,limit=1,sort=nearest] Passengers" 2>/dev/null)
BEFORE=$(rcon "data get entity @e[type=oak_boat,limit=1,sort=nearest] Pos" 2>/dev/null)
sleep 11
AFTER=$(rcon "data get entity @e[type=oak_boat,limit=1,sort=nearest] Pos" 2>/dev/null)
ROTATION=$(rcon "data get entity @e[type=oak_boat,limit=1,sort=nearest] Rotation" 2>/dev/null)

wait "$CLIENT_PID" 2>/dev/null
RC=$?

echo "== boat Pos before (mounted, at rest)"
echo "$BEFORE"
echo "== boat passengers"
echo "$MOUNTED"
echo "== boat Pos after (W 3 s, A 1 s)"
echo "$AFTER"
echo "== boat Rotation after"
echo "$ROTATION"
echo "== haste exit code $RC"
tail -3 "$LOG" | cut -c1-200

# The horizontal distance the boat covered between the two reads.
read_positions() { echo "$1" | grep -oE '\[[-0-9.]+d, [-0-9.]+d, [-0-9.]+d\]' | tr -d '[]d' | tr ',' ' '; }
set -- $(read_positions "$BEFORE")
BX=${1:-} BY=${2:-} BZ=${3:-}
set -- $(read_positions "$AFTER")
AX=${1:-} AY=${2:-} AZ=${3:-}
MOVED=$(awk -v bx="$BX" -v bz="$BZ" -v ax="$AX" -v az="$AZ" 'BEGIN {
  if (bx == "" || ax == "") { print "unknown"; exit }
  d = sqrt((ax - bx) * (ax - bx) + (az - bz) * (az - bz))
  printf "%.2f blocks", d
}')
echo "== boat travelled: ${MOVED}"

FAIL=
[ "$RC" -eq 0 ] || FAIL="${FAIL:+$FAIL; }haste exited $RC"
echo "$MOUNTED" | grep -q "$NAME" || FAIL="${FAIL:+$FAIL; }the boat did not carry $NAME"
case "$MOVED" in unknown) FAIL="${FAIL:+$FAIL; }no Pos answers over RCON";; esac
awk -v m="${MOVED% blocks}" 'BEGIN { exit !(m + 0 > 3) }' || FAIL="${FAIL:+$FAIL; }the boat barely moved (${MOVED})"

if [ -n "$SERVER_LOG" ]; then
  while IFS= read -r line; do
    echo "server: $line"
    FAIL="${FAIL:+$FAIL; }server: $line"
  done < <(tail -n +"$((SERVER_LINES + 1))" "$SERVER_LOG" 2>/dev/null |
    grep -iE 'moved wrongly|moved too quickly|kicked|Exception' || true)
else
  FAIL="${FAIL:+$FAIL; }server log not found, so the movement checks are unchecked"
fi

if [ -n "$FAIL" ]; then
  echo "FAIL: $FAIL"
  exit 1
fi
echo PASS
