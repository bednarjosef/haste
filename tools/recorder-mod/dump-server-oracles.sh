#!/usr/bin/env bash
# Builds the recorder mod, runs the 26.3 dedicated server headless with it installed, and
# produces the server-side oracle dumps in cache/oracle/26.3/:
#   block_shapes.json, entity_dimensions.json, block_properties.json
#
# The server runs OFFLINE on 127.0.0.1, flat world, and shuts itself down right after the dump
# (the mixin stops the server once dumping is done). The working server copy lives in
# cache/oracle-server so the spike's own cache/server-spike is never touched.
#
# Usage: tools/recorder-mod/dump-server-oracles.sh [--keep]
#   --keep  do not stop the server afterwards (debugging)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
JDK="${HASTE_JAVA:-$CACHE/jdk/bin}"
[ -x "$JDK/java" ] || JDK="$(dirname "$(command -v java)")"
export JAVA_HOME="${JAVA_HOME:-$(cd "$JDK/.." && pwd)}"

OUT="$CACHE/oracle/26.3"
SRV="$CACHE/oracle-server"

echo "== build mod"
"$HERE/build.sh" >/dev/null

echo "== prepare server dir ($SRV)"
mkdir -p "$SRV"
cp -f "$CACHE/server-spike/fabric-server-launch.jar" "$SRV/"
cp -f "$CACHE/server-spike/fabric-server-launcher.properties" "$SRV/"
cp -rf "$CACHE/server-spike/libraries" "$SRV/libraries"
# fabric-server-launch needs the vanilla server jar (its launcher properties point at server.jar)
# and its de-bundled copy under versions/.
cp -f "$CACHE/server-spike/server.jar" "$SRV/"
mkdir -p "$SRV/versions"
cp -rf "$CACHE/server-spike/versions/." "$SRV/versions/"
mkdir -p "$SRV/mods"
cp -f "$CACHE/server-spike/mods/fabric-api"*.jar "$SRV/mods/" 2>/dev/null || true
cp -f "$HERE/build/haste-recorder.jar" "$SRV/mods/"

echo "eula=true" > "$SRV/eula.txt"
cat > "$SRV/server.properties" <<'PROPS'
online-mode=false
server-ip=127.0.0.1
server-port=25599
level-type=minecraft\:flat
level-name=oracle
gamemode=creative
spawn-protection=0
max-players=1
motd=haste-oracle
sync-chunk-writes=false
pause-when-empty-seconds=-1
view-distance=2
simulation-distance=2
initial-enabled-packs=vanilla
PROPS

echo "== run server (dumps on first tick, then stops)"
mkdir -p "$OUT"
# The wait below looks for the three files, so the previous run's copies must go first.
rm -f "$OUT/block_shapes.json" "$OUT/entity_dimensions.json" "$OUT/block_properties.json"
cd "$SRV"
"$JDK/java" -Xms512M -Xmx1G -Dhaste.oracle.out="$OUT" -jar fabric-server-launch.jar nogui > "$SRV/oracle-run.log" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

# Wait for the three files (the mixin stops the server itself when done).
for i in $(seq 1 180); do
  [ -f "$OUT/block_shapes.json" ] && [ -f "$OUT/entity_dimensions.json" ] && [ -f "$OUT/block_properties.json" ] && break
  if ! kill -0 $SERVER_PID 2>/dev/null; then break; fi
  sleep 1
done

if [ "${1:-}" != "--keep" ]; then
  kill $SERVER_PID 2>/dev/null || true
  wait $SERVER_PID 2>/dev/null || true
fi
trap - EXIT

echo "== results"
ls -la "$OUT" || true
for f in block_shapes.json entity_dimensions.json block_properties.json; do
  if [ -f "$OUT/$f" ]; then
    echo "$f: $(wc -c < "$OUT/$f") bytes, $(wc -l < "$OUT/$f") lines"
  else
    echo "MISSING: $OUT/$f (see $SRV/oracle-run.log)" >&2
    tail -40 "$SRV/oracle-run.log" >&2 || true
    exit 1
  fi
done
