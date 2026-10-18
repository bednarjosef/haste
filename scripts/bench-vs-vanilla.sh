#!/usr/bin/env bash
# Haste against the real vanilla 26.3 client, both in real windows on this desktop: the same
# server, spot, view, window size and options.txt, one render distance after another.
# For each render distance it runs Haste's windowed bench (`--bench-window --x11`) and then
# vanilla (scripts/vanilla-client.sh with the bench_spot scenario), and samples each client's
# memory once a second: RSS (Haste's includes its embedded JVM) and GPU memory (amdgpu/i915
# fdinfo: VRAM + GTT, the GPU's share of system RAM on an integrated GPU).
#
# usage: scripts/bench-vs-vanilla.sh [port] [seconds] ["render distances"]
#        (defaults 25620, 30, "8 12 16 32")
# Needs the reference server running with a view distance at least the largest render
# distance: HASTE_REF_VIEW_DISTANCE=32 scripts/vanilla-server.sh &, then
# scripts/vanilla-rcon.sh setup. Run once before measuring so the server has generated the
# area (the first visit generates chunks, which would slow whichever client comes first).
# env: HASTE_BENCH_WIDTH/HEIGHT (default 1600x900), HASTE_BENCH_RUNS (default 1: runs per
#      client per render distance, alternating Haste and vanilla)
# Output: target/bench-vs-vanilla/<time>/ (per-run JSON, logs, memory samples, summary.txt).
set -euo pipefail
PORT=${1:-25620}
SECS=${2:-30}
RDS=${3:-"8 12 16 32"}
W=${HASTE_BENCH_WIDTH:-1600}
H=${HASTE_BENCH_HEIGHT:-900}
RUNS=${HASTE_BENCH_RUNS:-1}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
OUT="$ROOT/target/bench-vs-vanilla/$(date +%Y%m%d-%H%M%S)"
GAME="$CACHE/bench-haste-game"
mkdir -p "$OUT" "$GAME"
cd "$ROOT"

# The fps_idle spot and view (tools/recorder-mod/scenarios/bench_spot.txt).
X=9.5 Y=71 Z=6.5 YAW=0 PITCH=10

echo "== building release haste and the recorder mod"
cargo build --release -q --bin haste
"$ROOT/tools/recorder-mod/build.sh" >/dev/null

# Samples <pid>'s memory once a second into <file> until it exits:
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

run_haste() {
  local rd=$1 tag=$2
  "$ROOT/scripts/vanilla-options.sh" "$GAME/options.txt" "$W" "$H" "renderDistance:$rd" maxFps:260
  echo "-- haste rd $rd ($tag), load $(cut -d' ' -f1-3 /proc/loadavg)"
  # Teleport the player to the spot while it joins (the bench waits for --pos).
  ( for _ in $(seq 120); do
      "$ROOT/scripts/vanilla-rcon.sh" --port "$PORT" "tp haste $X $Y $Z $YAW $PITCH" >/dev/null 2>&1 || true
      sleep 1
    done ) &
  local tp=$!
  HASTE_GAME_DIR="$GAME" target/release/haste --server "127.0.0.1:$PORT" --username haste \
    --bench-window "$SECS" --bench-out "$OUT/haste-rd$rd-$tag.json" --x11 \
    --width "$W" --height "$H" --render-distance "$rd" \
    --pos "$X" "$Y" "$Z" --yaw "$YAW" --pitch "$PITCH" --seconds 240 \
    > "$OUT/haste-rd$rd-$tag.log" 2>&1 &
  local pid=$!
  sample_memory "$pid" "$OUT/haste-rd$rd-$tag.mem" &
  local sampler=$!
  wait "$pid" || echo "haste exited with $? (see $OUT/haste-rd$rd-$tag.log)"
  kill "$tp" 2>/dev/null || true; wait "$tp" 2>/dev/null || true
  wait "$sampler" 2>/dev/null || true
  grep -E "^frame interval|fps|FPS" "$OUT/haste-rd$rd-$tag.log" | tail -2 || true
}

run_vanilla() {
  local rd=$1 tag=$2
  echo "-- vanilla rd $rd ($tag), load $(cut -d' ' -f1-3 /proc/loadavg)"
  rm -f "$CACHE/oracle/26.3/bench/vanilla.json"
  HASTE_VANILLA_NO_BUILD=1 HASTE_VANILLA_WIDTH=$W HASTE_VANILLA_HEIGHT=$H \
    HASTE_VANILLA_OPTIONS="renderDistance:$rd maxFps:260" \
    "$ROOT/scripts/vanilla-client.sh" "127.0.0.1:$PORT" "$OUT/bench_spot.txt" 400 \
    > "$OUT/vanilla-rd$rd-$tag.log" 2>&1 &
  local launcher=$! pid=""
  for _ in $(seq 60); do
    # The JVM itself, not the `timeout` wrapper with the same command line.
    for p in $(pgrep -f "KnotClient.*--gameDir $CACHE/vanilla-reference/game" || true); do
      [ "$(cat "/proc/$p/comm" 2>/dev/null)" = java ] && pid=$p
    done
    [ -n "$pid" ] && break
    sleep 1
  done
  if [ -n "$pid" ]; then
    sample_memory "$pid" "$OUT/vanilla-rd$rd-$tag.mem" &
  fi
  wait "$launcher" || echo "vanilla exited with $? (see $OUT/vanilla-rd$rd-$tag.log)"
  wait 2>/dev/null || true
  if [ -f "$CACHE/oracle/26.3/bench/vanilla.json" ]; then
    cp "$CACHE/oracle/26.3/bench/vanilla.json" "$OUT/vanilla-rd$rd-$tag.json"
  fi
  grep -h "frametimes" "$OUT/vanilla-rd$rd-$tag.log" | tail -1 || true
}

# The bench_spot scenario with this run's measuring time.
sed "s/^frametimes 30 /frametimes $SECS /" "$ROOT/tools/recorder-mod/scenarios/bench_spot.txt" > "$OUT/bench_spot.txt"

for rd in $RDS; do
  for run in $(seq "$RUNS"); do
    run_haste "$rd" "$run"
    run_vanilla "$rd" "$run"
  done
done

python3 -I - "$OUT" "$SECS" <<'PY' | tee "$OUT/summary.txt"
import glob, json, os, re, statistics, sys
out, secs = sys.argv[1], int(sys.argv[2])
def mem(path):
    rows = []
    if os.path.exists(path):
        for line in open(path):
            p = line.split()
            if len(p) == 3:
                rows.append((int(p[0]), int(p[1]), int(p[2])))
    if not rows:
        return None
    # The measurement is the run's last SECS seconds (the client quits right after it).
    tail = rows[-secs:]
    return (max(r[1] for r in rows) / 1024, statistics.median(r[1] for r in tail) / 1024,
            max(r[2] for r in rows) / 1024)
print(f"{'client':8} {'rd':>3} {'run':>3} {'FPS':>6} {'avg ms':>7} {'p50':>6} {'p99':>6} {'p99.9':>6} {'worst':>7} "
      f"{'RSS MB':>7} {'peak':>6} {'GPU MB':>7}")
rows = []
for path in sorted(glob.glob(f"{out}/*-rd*-*.json")):
    name = os.path.basename(path)[:-5]
    client, rd, run = re.match(r"(haste|vanilla)-rd(\d+)-(\d+)", name).groups()
    d = json.load(open(path))
    if client == "haste":
        k = lambda f: d.get("interval_" + f, 0.0)
        vals = (k("fps"), k("avg_ms"), k("p50_ms"), k("p99_ms"), k("p999_ms"), k("worst_ms"))
    else:
        vals = (d["fps"], d["avg_ms"], d["p50_ms"], d["p99_ms"], d["p999_ms"], d["worst_ms"])
    m = mem(f"{out}/{name}.mem") or (0, 0, 0)
    rows.append((int(rd), client, int(run), vals, m))
for rd, client, run, v, m in sorted(rows):
    print(f"{client:8} {rd:>3} {run:>3} {v[0]:>6.0f} {v[1]:>7.3f} {v[2]:>6.2f} {v[3]:>6.2f} {v[4]:>6.2f} {v[5]:>7.2f} "
          f"{m[1]:>7.0f} {m[0]:>6.0f} {m[2]:>7.0f}")
print("FPS and ms: whole-frame time (Haste: interval between frames; vanilla: runTick to runTick)."
      " RSS MB: median over the measured seconds; peak: whole run. GPU MB: peak VRAM + GTT.")
PY
echo "results in $OUT"
