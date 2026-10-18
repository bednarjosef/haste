#!/usr/bin/env bash
# Suffocation push-out live check (scripts/checks/suffocation-push.txt) against the running
# test server on 127.0.0.1:25691. It builds a small stone platform high above the terrain in
# this branch's own area (x 555..565, z 5..15, y 249), stands the player on it, RCON
# /setblock sand at the player's feet and head, and reads the server's own position for the
# player over the next 20 ticks: `LocalPlayer.moveTowardsClosestSpace` must push the player
# off the sand column (net +x here). The rig is removed afterwards. No server is started and
# nothing outside this branch's area is touched.
# usage: scripts/checks/suffocation-push.sh
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=SuffPush
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
SCRIPT="$ROOT/scripts/checks/suffocation-push.txt"
LOG="$ROOT/target/suffocation-push.log"
cd "$ROOT" || exit 1
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }
read_pos() { rcon "data get entity $NAME Pos" 2>/dev/null | grep -oE '\[[-0-9.]+d, [-0-9.]+d, [-0-9.]+d\]'; }

: >"$LOG"
echo "== joining headless as $NAME with $SCRIPT"
"$BIN" --server "127.0.0.1:$PORT" --username "$NAME" --script "$SCRIPT" --seconds 60 >"$LOG" 2>&1 &
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

# The rig lands inside the script's `wait 400` (20 s): a stone platform at y 249 with the
# player's feet at y 250, then sand at the feet and head. Sand on a solid floor does not fall,
# so the player stays embedded until the push-out frees them.
rcon "execute at $NAME run fill 555 248 5 565 251 15 air" >/dev/null
rcon "execute at $NAME run fill 555 249 5 565 249 15 stone" >/dev/null
rcon "tp $NAME 560.5 250 10.5 0 0" >/dev/null
sleep 3
REST=$(read_pos)
echo "== player at rest: ${REST:-<no answer>}"

rcon "execute at $NAME run setblock 560 250 10 sand" >/dev/null
rcon "execute at $NAME run setblock 560 251 10 sand" >/dev/null
P0=$(read_pos)
sleep 0.2
P1=$(read_pos)
sleep 0.3
P2=$(read_pos)
sleep 0.5
P3=$(read_pos)
echo "== over the next 20 ticks (20 Hz)"
echo "t+0.0: ${P0:-<no answer>}"
echo "t+0.2: ${P1:-<no answer>}"
echo "t+0.5: ${P2:-<no answer>}"
echo "t+1.0: ${P3:-<no answer>}"

wait "$CLIENT_PID" 2>/dev/null
RC=$?

# Clean up exactly the rig we placed.
rcon "execute at $NAME run fill 555 248 5 565 251 15 air" >/dev/null
echo "== haste exit code $RC"
tail -3 "$LOG" | cut -c1-200

read_xz() { echo "$1" | grep -oE '[-0-9.]+' | tr '\n' ' '; }
set -- $(read_xz "$REST")
BX=${1:-} BZ=${3:-}
set -- $(read_xz "$P3")
AX=${1:-} AZ=${3:-}
FAIL=
[ "$RC" -eq 0 ] || FAIL="${FAIL:+$FAIL; }haste exited $RC"
if [ -z "$BX" ] || [ -z "$AX" ]; then
  FAIL="${FAIL:+$FAIL; }no Pos answers over RCON"
else
  DX=$(awk -v b="$BX" -v a="$AX" 'BEGIN { printf "%.2f", a - b }')
  DZ=$(awk -v b="$BZ" -v a="$AZ" 'BEGIN { printf "%.2f", a - b }')
  echo "== displacement: dx=$DX dz=$DZ"
  awk -v d="$DX" 'BEGIN { exit !(d + 0 > 0.2) }' || FAIL="${FAIL:+$FAIL; }not pushed east (dx=$DX)"
fi

if [ -n "$FAIL" ]; then
  echo "FAIL: $FAIL"
  exit 1
fi
echo PASS
