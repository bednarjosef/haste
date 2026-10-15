#!/usr/bin/env bash
# Horse riding live check (scripts/checks/horse-riding.txt) against the running test
# server on 127.0.0.1:25691. It builds a stone platform in this branch's own area
# (x 400..420, z 0..20, far from the shared spawn floor), summons a tamed, saddled horse,
# lets the scripted client mount it (use-item on the entity), ride forward (W, 3 s) and
# jump (Space, a charged jump bar), then reads the server's own verdict back over RCON:
# the horse's Pos before and after, that it carried the player, and that the server log
# holds no "moved wrongly"/"moved too quickly" for it. No server is started, and nothing
# outside this branch's area is touched.
# usage: scripts/checks/horse-riding.sh
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=HorseCheck
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
SCRIPT="$ROOT/scripts/checks/horse-riding.txt"
LOG="$ROOT/target/horse-riding.log"
cd "$ROOT" || exit 1
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }

# The running server's own log (haste-cache/servers/lead-normal; fall back to scanning the
# java process whose working directory names our port, as boat-steering.sh does).
SERVER_LOG=$HASTE_CACHE/servers/lead-normal/logs/latest.log
if [ ! -f "$SERVER_LOG" ]; then
  for p in /proc/[0-9]*; do
    cwd=$(readlink "$p/cwd" 2>/dev/null) || continue
    if [ -f "$cwd/server.properties" ] && grep -q "^server-port=$PORT\$" "$cwd/server.properties" 2>/dev/null; then
      SERVER_LOG="$cwd/logs/latest.log"
    fi
  done
fi
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
# the area (so the server loads its chunks), the platform and the horse summon (tamed, with
# the saddle on, so our client's `getControllingPassenger` holds from the mount).
rcon "tp $NAME 405.5 71 5.0 0 20" >/dev/null
sleep 4
# Clear the volume above the platform first: the area's natural terrain can be higher
# than y 71, which would bury the player (the crosshair then hits the block around the
# eyes and no use click ever reaches the horse).
rcon "execute at $NAME run fill 400 71 0 420 90 20 air" >/dev/null
rcon "execute at $NAME run fill 400 70 0 420 70 20 stone" >/dev/null
rcon "kill @e[type=horse]" >/dev/null
# NoAI: a free horse wanders off its mounting spot while the script's first wait runs
# (its AI ticks even from a distance); NoAI does not affect the ridden travel we steer.
rcon "summon minecraft:horse 405.5 71 8.5 {Tame:1b,NoAI:1b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}" >/dev/null
sleep 3
echo "== mounting spot"
SPOT=$(rcon "data get entity $NAME Pos" 2>/dev/null)
echo "player spot: $SPOT"
HORSE_REST=$(rcon "data get entity @e[type=horse,limit=1,sort=nearest] Pos" 2>/dev/null)
echo "horse rest: $HORSE_REST"

# The script's `click right` steps mount the horse (the second is a spare: the first mounts,
# a mounted rider has no crosshair target, so it sends nothing), then `wait 120` (6 s)
# holds it at rest. The reads must land after the mount and after the ride: the clicks
# happen ~21 s into the script (the first `wait 400` is the RCON setup window), W runs
# 27..30 s, the charged jump ~32 s, so the mounted read goes out at setup+16 s and the
# after-read at setup+33 s.
sleep 16
MOUNTED=$(rcon "data get entity $NAME RootVehicle" 2>/dev/null)
BEFORE=$(rcon "data get entity @e[type=horse,limit=1,sort=nearest] Pos" 2>/dev/null)
sleep 17
AFTER=$(rcon "data get entity @e[type=horse,limit=1,sort=nearest] Pos" 2>/dev/null)
ROTATION=$(rcon "data get entity @e[type=horse,limit=1,sort=nearest] Rotation" 2>/dev/null)
PLAYER_AFTER=$(rcon "data get entity $NAME Pos" 2>/dev/null)

wait "$CLIENT_PID" 2>/dev/null
RC=$?

echo "== horse Pos before (mounted, at rest)"
echo "$BEFORE"
echo "== horse passengers"
echo "$MOUNTED"
echo "== horse Pos after (W 3 s, a charged jump)"
echo "$AFTER"
echo "== horse Rotation after"
echo "$ROTATION"
echo "== haste exit code $RC"
tail -3 "$LOG" | cut -c1-200

# The horizontal distance the horse covered between the two reads.
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
echo "== horse travelled: ${MOVED}"

FAIL=
[ "$RC" -eq 0 ] || FAIL="${FAIL:+$FAIL; }haste exited $RC"
echo "$MOUNTED" | grep -q "minecraft:horse" || FAIL="${FAIL:+$FAIL; }the horse did not carry $NAME"
echo "== player Pos after"
echo "$PLAYER_AFTER"
case "$MOVED" in unknown) FAIL="${FAIL:+$FAIL; }no Pos answers over RCON";; esac
awk -v m="${MOVED% blocks}" 'BEGIN { exit !(m + 0 > 3) }' || FAIL="${FAIL:+$FAIL; }the horse barely moved (${MOVED})"

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
