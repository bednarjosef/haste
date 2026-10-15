#!/usr/bin/env bash
# The key-rebinding feature check, end to end and the way vanilla does it: the player
# rebinds Jump -> J and Drop Selected Item -> K in Options > Controls > Key Binds (the real
# vanilla KeyBindsScreen: click a change button, press a key, KeyMapping.setKey), closes,
# and the controls the client reads are the rebind ones — the server sees the player jump
# on J and the held stack drop by one on K. The bindings are then checked to persist in
# options.txt across a client restart, and Reset All restores the vanilla defaults.
# usage: scripts/checks/keybinds-check.sh
#
# Evidence: the client log (no "haste proxy:"/"Java exception"), the options.txt lines, and
# RCON `data get entity` answers: Pos y rising after J, the SelectedItem count after K.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT=25691
NAME=KeybindCheck
BIN="${HASTE_BIN:-$ROOT/target/debug/haste}"
LOGDIR="$ROOT/target/feature-checks"
mkdir -p "$LOGDIR"
rcon() { "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "$@"; }

PASS=0
FAIL=0
FAILED=()
check() { if [ "$3" = 0 ]; then echo "PASS $1: $2"; PASS=$((PASS + 1)); else echo "FAIL $1: $2"; FAIL=$((FAIL + 1)); FAILED+=("$1"); fi; }
has() { echo "$1" | grep -q -- "$2"; }
pos_y() { echo "$1" | grep -oE '\[[-0-9.]+d, [-0-9.]+d, [-0-9.]+d\]' | tr -d '[]d ' | cut -d, -f2; }
# True when both values are plain numbers (an unset position must not compare as one).
numeric() { case "$1$2" in ''|*[!0-9.-]*) return 1;; *) return 0;; esac; }

CLIENT_PID=""
OPTS=""
client_start() { # script log
  : >"$2"
  timeout 240 "$BIN" --server "127.0.0.1:$PORT" --username "$NAME" --script "$1" \
    --seconds 150 >"$2" 2>&1 &
  CLIENT_PID=$!
  # The game directory vanilla reads and writes options.txt in (JvmConfig::from_cache:
  # $HASTE_CACHE/jvm-game); read the cache the client process actually uses. The client
  # leaves as soon as the script's last step ran, so this must not wait — the first read
  # finds the process alive, and a dead one just means the default cache is used.
  OPTS=$(tr '\0' '\n' <"/proc/$CLIENT_PID/environ" 2>/dev/null | sed -n 's/^HASTE_CACHE=//p' | head -1)
  OPTS="${OPTS:-$ROOT/cache}/jvm-game/options.txt"
}

wait_online() {
  for _ in $(seq 1 300); do
    rcon list 2>/dev/null | grep -q "$NAME" && return 0
    sleep 0.5
  done
  return 1
}

wait_shot() { # log name [timeout_s]
  local log=$1 name=$2 t=${3:-90}
  for _ in $(seq 1 $((t * 10))); do
    grep -q "script-$name.png" "$log" 2>/dev/null && return 0
    sleep 0.1
  done
  return 1
}

wait_exit() { wait "$CLIENT_PID"; return $?; }

# Polls a predicate shell snippet against options.txt until it holds (the save lands a tick
# after the screen closes).
wait_options() { # description snippet
  local desc=$1 snippet=$2
  for _ in $(seq 1 75); do
    if [ -f "$OPTS" ] && eval "$snippet"; then return 0; fi
    sleep 0.2
  done
  echo "options.txt never satisfied: $desc ($OPTS)"
  return 1
}

# The highest Pos y over the next ~3 s (the jump apex).
max_y() { # log
  local y best="" v
  for _ in $(seq 1 15); do
    v=$(pos_y "$(rcon "data get entity $NAME Pos")" 2>/dev/null)
    [ -n "$v" ] && { [ -z "$best" ] || awk -v a="$v" -v b="$best" 'BEGIN{exit !(a>b)}' && best=$v; }
    sleep 0.2
  done
  echo "${best:-?}"
}

clean_log() { # log
  if grep -qE "haste proxy:|Java exception" "$1"; then
    echo "proxy stubs or Java exceptions:"; grep -E "haste proxy:|Java exception" "$1" | head -5
    return 1
  fi
  return 0
}

# ---- run 1: rebind Jump -> J and Drop -> K in Key Binds, then prove them live ----
RUN1="$LOGDIR/keybinds.log"
# A previous run (or its crash) may have left rebinds in options.txt: remove the key
# lines so run 1 starts from the vanilla defaults and its own clicks find "Space"/"Q".
sed -i '/^key_key\./d' "$ROOT/cache/jvm-game/options.txt" 2>/dev/null
client_start "$ROOT/scripts/checks/keybinds.txt" "$RUN1"
wait_online || { check run1 "the client joined the test server" 1; exit 1; }
rcon "gamemode survival $NAME" >/dev/null
rcon "tp $NAME 0 100 0 0 0" >/dev/null
sleep 3
if wait_shot "$RUN1" kb-joined 90; then
  : # joined; the rebind steps are driven by click_text and cannot partially apply
fi
wait_shot "$RUN1" kb-rebound 120 || check run1 "the Key Binds screen never opened (no kb-rebound shot)" 1
echo "--- run 1: options.txt after the rebinds ---"
wait_options "key_key.jump = key.keyboard.j and key_key.drop = key.keyboard.k" \
  'grep -q "^key_key.jump:key.keyboard.j$" "$OPTS" && grep -q "^key_key.drop:key.keyboard.k$" "$OPTS"'
REBIND_RC=$?
grep -E "^key_key\.(jump|drop):" "$OPTS" 2>/dev/null || echo "<options.txt not found at $OPTS>"
check run1 "options.txt holds key_key.jump:key.keyboard.j and key_key.drop:key.keyboard.k" $REBIND_RC
Y0=$(pos_y "$(rcon "data get entity $NAME Pos")")
rcon "item replace entity $NAME weapon.mainhand with minecraft:stone 3" >/dev/null
BEFORE=$(rcon "data get entity $NAME SelectedItem")
echo "--- run 1: live rebinding evidence ---"
echo "Pos y before J:   $Y0"
echo "SelectedItem before K: $BEFORE"
if wait_shot "$RUN1" kb-jump 60; then
  Y1=$(max_y "$RUN1")
  echo "Pos y peak after J: $Y1"
  numeric "$Y0" "$Y1" && awk -v a="$Y1" -v b="$Y0" 'BEGIN{exit !(a>b+0.3)}' \
    && check run1 "J made the player jump (y $Y0 -> $Y1)" 0 \
    || check run1 "J did not make the player jump (y $Y0 -> $Y1)" 1
else
  check run1 "the jump step never ran (no kb-jump shot)" 1
fi
if wait_shot "$RUN1" kb-drop 60; then
  sleep 2
  AFTER=$(rcon "data get entity $NAME SelectedItem")
  echo "SelectedItem after K:  $AFTER"
  has "$BEFORE" "stone" && has "$BEFORE" "count: 3" || { echo "no 3 stone in hand before K"; check run1 "K dropped one item (held 3 stone, dropped to 2)" 1; }
  has "$AFTER" "stone" && has "$AFTER" "count: 2" \
    && check run1 "K dropped one item (held 3 stone, dropped to 2)" 0 \
    || check run1 "K did not drop one item (SelectedItem: $AFTER)" 1
else
  check run1 "the drop step never ran (no kb-drop shot)" 1
fi
wait_exit
RC1=$?
[ "$RC1" = 0 ] && check run1 "the script finished cleanly" 0 || check run1 "the client exited $RC1" "$RC1"
clean_log "$RUN1" && check run1 "no proxy stubs or Java exceptions in the client log" 0 \
  || check run1 "the client log has proxy stubs or Java exceptions" 1

# ---- run 2 (the restarted client): the persisted bindings work, then Reset All ----
RUN2="$LOGDIR/keybinds-restart.log"
client_start "$ROOT/scripts/checks/keybinds-restart.txt" "$RUN2"
wait_online || { check run2 "the restarted client joined the test server" 1; exit 1; }
rcon "gamemode survival $NAME" >/dev/null
rcon "tp $NAME 0 100 0 0 0" >/dev/null
if wait_shot "$RUN2" r-joined 90; then
  sleep 3
  if wait_shot "$RUN2" r-stand 30; then
    Y0=$(pos_y "$(rcon "data get entity $NAME Pos")")
    echo "--- run 2: the restarted client loads options.txt ---"
    echo "Pos y before J:   $Y0"
    if wait_shot "$RUN2" r-jump 60; then
      Y1=$(max_y "$RUN2")
      echo "Pos y peak after J: $Y1"
      numeric "$Y0" "$Y1" && awk -v a="$Y1" -v b="$Y0" 'BEGIN{exit !(a>b+0.3)}' \
        && check run2 "J still jumps after the restart (y $Y0 -> $Y1)" 0 \
        || check run2 "J does not jump after the restart (y $Y0 -> $Y1)" 1
    else
      check run2 "the post-restart jump step never ran (no r-jump shot)" 1
    fi
  else
    check run2 "the player never stood still after the restart (no r-stand shot)" 1
  fi
fi
wait_shot "$RUN2" r-reset 120 || check run2 "the Key Binds screen never reopened (no r-reset shot)" 1
echo "--- run 2: options.txt after Reset All ---"
wait_options "key_key.jump = key.keyboard.space and key_key.drop = key.keyboard.q" \
  'grep -q "^key_key.jump:key.keyboard.space$" "$OPTS" && grep -q "^key_key.drop:key.keyboard.q$" "$OPTS"'
RESET_RC=$?
grep -E "^key_key\.(jump|drop):" "$OPTS" 2>/dev/null || echo "<options.txt not found at $OPTS>"
check run2 "Reset All restored key_key.jump:key.keyboard.space and key_key.drop:key.keyboard.q" $RESET_RC
wait_exit
RC2=$?
[ "$RC2" = 0 ] && check run2 "the script finished cleanly" 0 || check run2 "the client exited $RC2" "$RC2"
clean_log "$RUN2" && check run2 "no proxy stubs or Java exceptions in the client log" 0 \
  || check run2 "the client log has proxy stubs or Java exceptions" 1

echo
echo "== $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ] || { echo "failed: ${FAILED[*]}"; exit 1; }
