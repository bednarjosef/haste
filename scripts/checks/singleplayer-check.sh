#!/usr/bin/env bash
# The singleplayer feature check: title -> Singleplayer -> world list -> Delete -> Create
# New World -> Create -> in game -> pause -> Save and Quit to Title, through the real local
# server. usage: scripts/checks/singleplayer-check.sh
#
# Steady state: the config dir's singleplayer/ holds a sentinel world ("Haste SP Sentinel",
# never deleted — it keeps the world list non-empty so vanilla never auto-jumps to the
# create screen) and this check's world ("Haste SP Check", deleted and recreated every run).
# A first run on a machine without them primes them here through the real flow first.
#
# Evidence: the client log (screens seen, "joined", "singleplayer server stopped") is grepped
# and the script screenshots' text lines are printed.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
SP="$HOME/.config/haste/singleplayer"
WORLD="Haste SP Check"
SENTINEL="Haste SP Sentinel"
LOG="$ROOT/target/feature-checks/singleplayer.log"
mkdir -p "$ROOT/target/feature-checks" "$SP"

# Creates `$1` through the real flow (create screen -> local server -> join -> Save and
# Quit), leaving it in the world list. `first` skips the Create New World click: on a
# machine with no worlds at all, vanilla's empty list jumps straight to the create screen.
prime() { # name [first]
  local name=$1 first=${2:-}
  local script="$ROOT/target/feature-checks/prime-$name.txt"
  {
    echo "wait_until screen TitleScreen"
    echo "wait 40"
    echo "click_text Singleplayer"
    if [ "$first" = first ]; then
      echo "wait_until screen CreateWorldScreen"
    else
      echo "wait_until screen SelectWorldScreen"
      echo "wait 40"
      echo "click_text Create New World"
      echo "wait_until screen CreateWorldScreen"
    fi
    for _ in 1 2 3 4 5 6 7 8 9; do echo "key BACKSPACE"; done
    echo "type $name"
    echo "click_text Create New World"
    echo "wait_until joined"
    echo "wait 200"
    echo "key ESCAPE"
    echo "wait_until screen_open"
    echo "click_text Save and Quit to Title"
    echo "wait_until screen TitleScreen"
  } >"$script"
  timeout 300 "$BIN" --script "$script" --seconds 280 >"$LOG.prime" 2>&1
}

# The steady state: sentinel first (on an empty dir the empty-list auto-jump opens the
# create screen by itself), then the check's world.
[ -f "$SP/$SENTINEL/level.dat" ] || prime "$SENTINEL" first
[ -f "$SP/$WORLD/level.dat" ] || prime "$WORLD"

timeout 300 "$BIN" --script "$ROOT/scripts/checks/singleplayer.txt" --seconds 280 >"$LOG" 2>&1
RC=$?

echo "== client exit: $RC"
grep -E "script (text|frame)\[sp-" "$LOG" || true
echo "== key log lines:"
grep -E "joined|creating singleplayer|singleplayer server stopped" "$LOG" | tail -5 || true

PASS=0
FAILED=()
check() { if [ "$2" = 0 ]; then echo "PASS $1"; PASS=$((PASS+1)); else echo "FAIL $1"; FAILED+=("$1"); fi; }

grep -q "joined 127.0.0.1" "$LOG"; check "the local world was joined" $?
grep -q "singleplayer server stopped" "$LOG"; check "the local server stopped on quit" $?
grep -q "script text\[sp-pause\]: Save and Quit to Title" "$LOG"; check "the pause screen says Save and Quit to Title" $?
grep -q "script text\[sp-back-at-title\]: Singleplayer" "$LOG"; check "back at the title screen" $?
[ "$RC" = 0 ]; check "the script finished cleanly" $?

echo
echo "== $PASS passed, ${#FAILED[@]} failed =="
[ ${#FAILED[@]} -eq 0 ] || { echo "failed: ${FAILED[*]}"; exit 1; }
