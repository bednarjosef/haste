#!/usr/bin/env bash
# Client rendering benchmark: builds release haste, joins the given server, waits for the world
# to settle, then renders offscreen at 1920x1080 along the fixed bench camera path
# (360-degree turn + straight flight) at render distance 12 and 32, and prints both summaries
# side by side. The same numbers are kept as JSON in target/bench/bench-<rd>.json.
# usage: scripts/bench.sh [server] [seconds]   (defaults 127.0.0.1:25691, 30)
# The server must already be running (e.g. scripts/test-server.sh); this script never starts
# or stops servers. Note: the server's own view-distance caps the effective render distance,
# which the summary reports as "render distance N (requested M)".
set -euo pipefail
SERVER=${1:-127.0.0.1:25691}
SECONDS_BENCH=${2:-30}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="$ROOT/target/bench"
mkdir -p "$OUT"

echo "== building release haste (first build takes a few minutes)"
cargo build --release -q --bin haste

for RD in 12 32; do
  echo "== benching render distance $RD ($SECONDS_BENCH s, 1920x1080, server $SERVER)"
  target/release/haste --bench "$SECONDS_BENCH" --bench-out "$OUT/bench-$RD.json" \
    --server "$SERVER" --render-distance "$RD" --width 1920 --height 1080 \
    --seconds $((SECONDS_BENCH + 60)) >"$OUT/bench-$RD.log" 2>"$OUT/bench-$RD.err"
  cat "$OUT/bench-$RD.log"
done

echo
echo "== side by side (rd 12 | rd 32)"
# Both logs have the same 8 lines (5 summary + blank lines); drop the trailing "written to"
# lines so the alignment holds even if one run wrote JSON and the other did not.
grep -v "^bench summary written to" "$OUT/bench-12.log" >"$OUT/bench-12.summary"
grep -v "^bench summary written to" "$OUT/bench-32.log" >"$OUT/bench-32.summary"
paste -d'|' "$OUT/bench-12.summary" "$OUT/bench-32.summary" | column -t -s'|'
