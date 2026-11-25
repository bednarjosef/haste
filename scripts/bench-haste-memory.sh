#!/usr/bin/env bash
# Haste's windowed bench (`--bench-window`) with the memory sampling of
# scripts/bench-vs-vanilla.sh, without the vanilla half: RSS and GPU memory (amdgpu/i915
# fdinfo: VRAM + GTT, the GPU's share of system RAM on an integrated GPU) sampled once a
# second while the bench runs, plus the client's own per-subsystem breakdown lines.
#
# usage: scripts/bench-haste-memory.sh <render distance> <seconds> <tag> [width] [height]
#        [username] ["x y z yaw pitch"] [extra haste args...]
# Needs the reference server (default port 25620) running with a view distance at least the
# render distance. Output: target/bench-haste-memory/<tag>/.
# env: HASTE_BENCH_SPOT ("x y z yaw pitch", default the screenshot spot below),
#      HASTE_BENCH_USER (default perfmem), HASTE_BENCH_PORT (default 25620).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
PORT="${HASTE_BENCH_PORT:-25620}"
RD=$1; SECS=$2; TAG=$3; W=${4:-1408}; H=${5:-828}; USER_ARG=${6:-}; SPOT_ARG=${7:-}
EXTRA=("${@:8}")
OUT="$ROOT/target/bench-haste-memory/$TAG"
mkdir -p "$OUT"
GAME="$CACHE/bench-haste-game"
if [ -n "$SPOT_ARG" ]; then HASTE_BENCH_SPOT="$SPOT_ARG"; fi
if [ -n "$USER_ARG" ]; then HASTE_BENCH_USER="$USER_ARG"; fi
read -r X Y Z YAW PITCH <<< "${HASTE_BENCH_SPOT:-9.5 71 6.5 0 10}"
cd "$ROOT"

"$ROOT/scripts/vanilla-options.sh" "$GAME/options.txt" "$W" "$H" "renderDistance:$RD" maxFps:260

# Teleport the bench player to the spot while it joins (the bench waits for --pos).
( for _ in $(seq 120); do
    "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "tp ${HASTE_BENCH_USER:-perfmem} $X $Y $Z $YAW $PITCH" >/dev/null 2>&1 || true
    sleep 1
done ) &
TP=$!

# Samples <pid>'s memory once a second into <file>:
# "<unix seconds> <rss KiB> <gpu KiB>", GPU memory summed over the process's DRM clients
# (every open file of one client repeats the client's numbers, so each counts once).
sample_memory() {
  local pid=$1 file=$2
  while kill -0 "$pid" 2>/dev/null; do
    local rss gpu
    rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
    gpu=$(awk '
      FNR == 1 { id = "" }
      /^drm-client-id:/ { id = $2 }
      /^drm-resident-(vram|gtt):/ { v = $2; if ($3 == "MiB") v *= 1024; if ($3 == "GiB") v *= 1048576; mem[id, $1] = v; seen[id] = 1 }
      END { s = 0; for (k in mem) s += mem[k]; print s + 0 }' "/proc/$pid/fdinfo/"* 2>/dev/null || echo 0)
    echo "$(date +%s) ${rss:-0} ${gpu:-0}" >> "$file"
    sleep 1
  done
}

HASTE_GAME_DIR="$GAME" "$ROOT/target/release/haste" --server "127.0.0.1:$PORT" \
  --username "${HASTE_BENCH_USER:-perfmem}" \
  --bench-window "$SECS" --bench-out "$OUT/haste-rd$RD.json" --x11 \
  --width "$W" --height "$H" --render-distance "$RD" \
  --pos "$X" "$Y" "$Z" --yaw "$YAW" --pitch "$PITCH" --seconds 100 \
  "${EXTRA[@]}" \
  > "$OUT/haste-rd$RD.log" 2>&1 &
PID=$!
# Watchdog, not a `timeout` wrapper: the sampler must see the client's own PID (timeout
# would sit between them and report its own tiny RSS).
( sleep 120; kill "$PID" 2>/dev/null ) &
WATCHDOG=$!
sample_memory "$PID" "$OUT/haste-rd$RD.mem" &
SAMPLER=$!
wait "$PID" || echo "haste exited with $? (see $OUT/haste-rd$RD.log)"
kill "$WATCHDOG" "$TP" 2>/dev/null || true; wait "$TP" "$WATCHDOG" 2>/dev/null || true
wait "$SAMPLER" 2>/dev/null || true

echo "== load $(cut -d' ' -f1-3 /proc/loadavg)"
echo "== frame interval:"
grep "^frame interval" "$OUT/haste-rd$RD.log" || true
echo "== memory (MiB): rss median over the last $SECS samples, peaks over the run"
tail -n "$SECS" "$OUT/haste-rd$RD.mem" | awk '{print $2}' | sort -n | \
  awk '{a[NR]=$1} END {printf "rss median %.0f\n", a[int(NR/2)+1]/1024}'
awk '{if ($2>mx) mx=$2; if ($3>mg) mg=$3} END {printf "rss peak %.0f, gpu peak %.0f\n", mx/1024, mg/1024}' "$OUT/haste-rd$RD.mem"
echo "== client memory breakdown:"
grep "^bench memory" "$OUT/haste-rd$RD.log" || true
