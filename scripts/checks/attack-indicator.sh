#!/usr/bin/env bash
# The attack indicator render smoke (review item: the crosshair recharge bar and the full icon
# on a mob). One client run drives scripts/checks/attack-indicator.txt: scripted attacks at the
# sky for the recharge bar, then a still zombie summoned 2.5 blocks ahead of the player (inside
# the 3-block pick reach) and a diamond sword given (a slow item: delay > vanilla
# Hud.MIN_CROSSHAIR_ATTACK_SPEED) for the full icon. The player runs wherever the fresh spawn
# put them (the shared camera spot at (9.5, 71, 6.5) has no ground under it: a camera spot, not
# a standing spot). Evidence: the debug hud-frame lines (the attack indicator sprite count per
# frame and the mirrored charge) and the screenshots in target/shots/.
# usage: scripts/checks/attack-indicator.sh
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=${HASTE_CHECK_NAME:-AtkInd$$}
NAME=${NAME:0:16}
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
LOG="$ROOT/target/feature-checks/attack-indicator.log"
mkdir -p "$(dirname "$LOG")"
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }
shot_seen() { grep -q "script-$1.png" "$LOG" 2>/dev/null; }

# The script waits for the hotbar item (the give) between its two phases.
timeout 200 env RUST_LOG=haste=debug "$BIN" --server "127.0.0.1:$PORT" --username "$NAME" \
  --script "$ROOT/scripts/checks/attack-indicator.txt" --seconds 120 >"$LOG" 2>&1 &
CLIENT_PID=$!

# Phase 2's scene, once the bar screenshots are done (the script has looked 2.5 blocks ahead
# and is waiting for the item): a still zombie there and the sword. Retried: on a busy shared
# server the player entity can lag the join, and RCON answers "No player was found" until it is.
for _ in $(seq 1 60); do
  rcon list 2>/dev/null | grep -q "$NAME" && break
  sleep 0.5
done
for _ in $(seq 1 120); do
  shot_seen indicator-bar-late && break
  sleep 0.5
done
rcon "execute at $NAME positioned ^ ^ ^2.5 run summon minecraft:zombie ~ ~ ~ {NoAI:1b,PersistenceRequired:1b}" >/dev/null
end=$((SECONDS + 25))
while [ $SECONDS -lt $end ]; do
  rcon "give $NAME minecraft:diamond_sword 1" >/dev/null
  rcon "data get entity $NAME Inventory" 2>/dev/null | grep -q "diamond_sword" && break
  sleep 2
done

for _ in $(seq 1 120); do
  shot_seen indicator-full-icon && break
  sleep 0.5
done
# Our zombie, removed while the player is still here to anchor the selector (scoped: never
# another worker's).
rcon "execute at $NAME run kill @e[type=zombie,distance=..4]" >/dev/null
wait "$CLIENT_PID"
r=$?

echo "== client exit $r, log $LOG"
echo "== screenshots (target/shots/script-indicator-*.png)"
ls -la "$ROOT/target/shots"/script-indicator-*.png 2>/dev/null || echo "none"
echo "== attack indicator sprites per frame (debug log; 0 = none, 2 = the bar, 1 = the full icon)"
grep "hud frame" "$LOG" | grep -v "attack 0" | sed 's/.*attack \(.\) charge AttackStrengthHud { ticker: \([0-9.]*\), delay: \([0-9.]*\) }.*/attack sprites \1, charge \2\/\3/' | uniq -c | tail -30
exit $r
