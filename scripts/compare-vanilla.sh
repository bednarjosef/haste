#!/usr/bin/env bash
# Takes the same terrain shots with Haste that the vanilla reference took, and compares them.
# usage: scripts/compare-vanilla.sh [name-prefix] [port]     (default prefix "terrain-", port 25620)
# Needs the reference server running (scripts/vanilla-server.sh + scripts/vanilla-rcon.sh setup)
# and the vanilla shots in $HASTE_CACHE/oracle/26.3/screenshots/<name>.{png,json}
# (scripts/vanilla-client.sh 127.0.0.1:25620 terrain).
# For each shot: Haste joins as "haste", the server teleports it to the recorded camera
# position (eye height 1.62 above the feet) while it settles, `haste --screenshot` renders at
# the recorded size, yaw and pitch. Output in target/vanilla-compare/: <name>.haste.png,
# <name>.compare.png (vanilla | haste | difference) and summary.txt (percent differing pixels,
# tolerance 8 per channel).
set -euo pipefail
PREFIX=${1:-terrain-}; PORT=${2:-25620}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
SHOTS="$CACHE/oracle/26.3/screenshots"
OUT="$ROOT/target/vanilla-compare"
mkdir -p "$OUT"
cd "$ROOT"
cargo build --release -j 6 --bin haste -q
cargo build --release -j 6 -p haste-testkit --example compare-screenshot -q
HASTE="$ROOT/target/release/haste"
COMPARE="$ROOT/target/release/examples/compare-screenshot"
SUMMARY="$OUT/summary.txt"
printf '%-32s %10s %10s %6s\n' shot differing% mean-err max > "$SUMMARY"
shopt -s nullglob
for json in "$SHOTS/$PREFIX"*.json; do
  name=$(basename "$json" .json)
  read -r X Y Z YAW PITCH W H < <(python3 -I -c '
import json, sys
d = json.load(open(sys.argv[1])); c = d["camera"]
print(c["pos"][0], c["pos"][1] - 1.62, c["pos"][2], c["yaw"], c["pitch"], d["width"], d["height"])' "$json")
  # Keep teleporting Haste to the spot while it joins and settles (a tp to the same spot
  # does not reload anything once it is there).
  ( for _ in $(seq 40); do
      "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "tp haste $X $Y $Z $YAW $PITCH" >/dev/null 2>&1 || true
      sleep 0.25
    done ) &
  TP=$!
  "$HASTE" --server "127.0.0.1:$PORT" --screenshot "$OUT/$name.haste.png" --hud off \
    --pos "$X" "$Y" "$Z" --width "$W" --height "$H" --yaw "$YAW" --pitch "$PITCH" --seconds 40 \
    > "$OUT/$name.haste.log" 2>&1 || echo "haste failed for $name (see $OUT/$name.haste.log)" >&2
  kill "$TP" 2>/dev/null || true; wait "$TP" 2>/dev/null || true
  grep -h '^screenshot ' "$OUT/$name.haste.log" || true
  if [ -f "$OUT/$name.haste.png" ]; then
    read -r PCT MEAN MAX < <("$COMPARE" "$SHOTS/$name.png" "$OUT/$name.haste.png" "$OUT/$name.compare.png" 8)
    printf '%-32s %10s %10s %6s\n' "$name" "$PCT" "$MEAN" "$MAX" >> "$SUMMARY"
  fi
done
cat "$SUMMARY"
