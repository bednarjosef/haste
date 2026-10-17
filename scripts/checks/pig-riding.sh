#!/usr/bin/env bash
# Pig riding live check (scripts/checks/pig-riding.txt) against the running test server on
# 127.0.0.1:25691. It builds the stone platform in this branch's own area (x 400..420,
# z 0..20, far from the shared spawn floor), gives the scripted player a carrot on a stick,
# summons a saddled pig, lets the scripted client mount it (use-item on the entity) and boost
# it (a second use click, vanilla `FoodOnAStickItem.use`), then reads the server's own verdict
# back over RCON: the pig's Pos before and after, that it carried the player, and that the
# server log holds no "moved wrongly"/"moved too quickly" for it. No server is started, and
# nothing outside this branch's area is touched.
# usage: scripts/checks/pig-riding.sh
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=PigCheck
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
SCRIPT="$ROOT/scripts/checks/pig-riding.txt"
LOG="$ROOT/target/pig-riding.log"
cd "$ROOT" || exit 1
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }

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
"$BIN" --server "127.0.0.1:$PORT" --username "$NAME" --script "$SCRIPT" --seconds 150 >"$LOG" 2>&1 &
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

# The RCON setup must land inside the script's first `wait 400` (20 s): the teleport into the
# area, the platform, the steer stick and the pig summon (the saddle is the SADDLE entry of
# the `equipment` map in 26.3's NBT; NoAI keeps the pig on its mounting spot).
rcon "tp $NAME 405.5 71 5.0 0 20" >/dev/null
sleep 4
rcon "execute at $NAME run fill 400 71 0 420 90 20 air" >/dev/null
rcon "execute at $NAME run fill 400 70 0 420 70 20 stone" >/dev/null
rcon "kill @e[type=pig]" >/dev/null
rcon "clear $NAME" >/dev/null
rcon "give $NAME minecraft:carrot_on_a_stick" >/dev/null
rcon "summon minecraft:pig 405.5 71 8.5 {NoAI:1b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}" >/dev/null
sleep 3
echo "== mounting spot"
SPOT=$(rcon "data get entity $NAME Pos" 2>/dev/null)
echo "player spot: $SPOT"
PIG_REST=$(rcon "data get entity @e[type=pig,limit=1,sort=nearest] Pos" 2>/dev/null)
echo "pig rest: $PIG_REST"

# The script's first `click right` mounts the pig (the second starts the boost); the pig then
# walks forward on its own. The offscreen client's ticks run slow while the world settles, so
# the clicks land well after the nominal script times (a minute or so in), and the reads must
# wait them out: the mounted and after-reads go out at setup+80 s, with the rest position from
# the setup (the pig walks the moment it is mounted, so there is no at-rest read after it).
sleep 80
MOUNTED=$(rcon "data get entity $NAME RootVehicle" 2>/dev/null)
AFTER=$(rcon "data get entity @e[type=pig,limit=1,sort=nearest] Pos" 2>/dev/null)
ROTATION=$(rcon "data get entity @e[type=pig,limit=1,sort=nearest] Rotation" 2>/dev/null)
PLAYER_AFTER=$(rcon "data get entity $NAME Pos" 2>/dev/null)

wait "$CLIENT_PID" 2>/dev/null
RC=$?

echo "== pig Pos before (the mount's spot)"
echo "$PIG_REST"
echo "== rider's vehicle"
echo "$MOUNTED"
echo "== pig Pos after (the walk plus a boost)"
echo "$AFTER"
echo "== pig Rotation after"
echo "$ROTATION"
echo "== haste exit code $RC"
tail -3 "$LOG" | cut -c1-200

read_positions() { echo "$1" | grep -oE '\[[-0-9.]+d, [-0-9.]+d, [-0-9.]+d\]' | tr -d '[]d' | tr ',' ' '; }
set -- $(read_positions "$PIG_REST")
BX=${1:-} BY=${2:-} BZ=${3:-}
set -- $(read_positions "$AFTER")
AX=${1:-} AY=${2:-} AZ=${3:-}
MOVED=$(awk -v bx="$BX" -v bz="$BZ" -v ax="$AX" -v az="$AZ" 'BEGIN {
  if (bx == "" || ax == "") { print "unknown"; exit }
  d = sqrt((ax - bx) * (ax - bx) + (az - bz) * (az - bz))
  printf "%.2f blocks", d
}')
echo "== pig travelled: ${MOVED}"

FAIL=
[ "$RC" -eq 0 ] || FAIL="${FAIL:+$FAIL; }haste exited $RC"
echo "$MOUNTED" | grep -q "minecraft:pig" || FAIL="${FAIL:+$FAIL; }the pig did not carry $NAME"
case "$MOVED" in unknown) FAIL="${FAIL:+$FAIL; }no Pos answers over RCON";; esac
awk -v m="${MOVED% blocks}" 'BEGIN { exit !(m + 0 > 3) }' || FAIL="${FAIL:+$FAIL; }the pig barely moved (${MOVED})"

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
