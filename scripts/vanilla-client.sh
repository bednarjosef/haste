#!/usr/bin/env bash
# Runs the real vanilla 26.3 client (Fabric Loader + Fabric API + the haste recorder mod)
# unattended: offline username, fixed window size and options.txt, joins <server> directly
# (quick play), runs a recorder scenario and quits by itself.
#
# usage: scripts/vanilla-client.sh <host:port> <scenario> [timeout-seconds]
#   <scenario>  a file name in tools/recorder-mod/scenarios/ (with or without .txt) or a path;
#               the scenario language is described in tools/recorder-mod/scenarios/README
#   timeout     hard kill after this many seconds (default 300); the scenario normally ends
#               with `quit` long before that
# env: HASTE_VANILLA_WIDTH/HEIGHT (default 1280x720), HASTE_VANILLA_USER (default HasteRef),
#      HASTE_VANILLA_VIDEO (SDL video driver, default x11),
#      HASTE_VANILLA_NO_BUILD=1 to skip rebuilding the recorder mod
#      HASTE_VANILLA_OPTIONS="key:value ..." extra options.txt lines (e.g. "renderDistance:12 maxFps:260")
# Output: whatever the scenario records, under $HASTE_CACHE/oracle/26.3/ (screenshots/, traces/);
# client log in $HASTE_CACHE/vanilla-reference/game/logs/latest.log.
# Never touches ~/.minecraft: the game dir lives under the cache.
set -euo pipefail
SERVER=${1:?usage: vanilla-client.sh <host:port> <scenario> [timeout-seconds]}
SCENARIO=${2:?scenario}
TIMEOUT=${3:-300}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
JAVA="$CACHE/jdk/bin/java"
WIDTH=${HASTE_VANILLA_WIDTH:-1280}
HEIGHT=${HASTE_VANILLA_HEIGHT:-720}
USERNAME=${HASTE_VANILLA_USER:-HasteRef}

case "$SCENARIO" in
  */*) SCENARIO_FILE="$(cd "$(dirname "$SCENARIO")" && pwd)/$(basename "$SCENARIO")" ;;
  *) SCENARIO_FILE="$ROOT/tools/recorder-mod/scenarios/${SCENARIO%.txt}.txt" ;;
esac
[ -f "$SCENARIO_FILE" ] || { echo "no scenario file $SCENARIO_FILE" >&2; exit 2; }

VERSION_JSON="$CACHE/versions/26.3.json"
FABRIC_JSON="$CACHE/fabric-game/fabric-loader-26.3.json"
for f in "$JAVA" "$VERSION_JSON" "$FABRIC_JSON" "$CACHE/versions/26.3/client.jar"; do
  [ -e "$f" ] || { echo "missing $f (run haste setup / the setup launcher's Fabric install first)" >&2; exit 2; }
done
FABRIC_API=$(ls "$CACHE"/server-spike/mods/fabric-api-*.jar 2>/dev/null | head -1)
[ -n "$FABRIC_API" ] || { echo "missing Fabric API jar in $CACHE/server-spike/mods" >&2; exit 2; }

if [ "${HASTE_VANILLA_NO_BUILD:-0}" != 1 ]; then
  "$ROOT/tools/recorder-mod/build.sh" >/dev/null
fi

GAME="$CACHE/vanilla-reference/game"
NATIVES="$CACHE/vanilla-reference/natives"
OUT="$CACHE/oracle/26.3"
mkdir -p "$GAME/mods" "$NATIVES" "$OUT"
rm -f "$GAME"/mods/*.jar
cp "$ROOT/tools/recorder-mod/build/haste-recorder.jar" "$GAME/mods/"
cp "$FABRIC_API" "$GAME/mods/"

# Classpath: Fabric's loader libraries first, then the version's Linux libraries, then the
# client jar (Knot finds the game jar on the classpath).
CP=$(python3 -I - "$VERSION_JSON" "$FABRIC_JSON" "$CACHE/libraries" "$CACHE/versions/26.3/client.jar" <<'PY'
import json, os, sys
version, fabric, libs, client = sys.argv[1:5]
def allowed(lib):
    rules = lib.get("rules")
    if not rules:
        return True
    ok = False
    for r in rules:
        osname = r.get("os", {}).get("name")
        if osname is None or osname == "linux":
            ok = r["action"] == "allow"
    return ok
paths, seen = [], set()
for lib in json.load(open(fabric))["libraries"]:
    g, a, v = lib["name"].split(":")[:3]
    paths.append(f"{libs}/{g.replace('.', '/')}/{a}/{v}/{a}-{v}.jar")
    seen.add((g, a))
for lib in json.load(open(version))["libraries"]:
    art = lib.get("downloads", {}).get("artifact")
    if not art or not allowed(lib):
        continue
    g, a = lib["name"].split(":")[:2]
    if (g, a) in seen and "natives" not in lib["name"]:
        continue  # Fabric ships its own (newer) ASM
    p = f"{libs}/{art['path']}"
    if os.path.isfile(p):
        paths.append(p)
paths.append(client)
print(":".join(paths))
PY
)
ASSET_INDEX=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["assetIndex"]["id"])' "$VERSION_JSON")

"$ROOT/scripts/vanilla-options.sh" "$GAME/options.txt" "$WIDTH" "$HEIGHT" ${HASTE_VANILLA_OPTIONS:-}

UUID=$(python3 -I -c 'import hashlib,sys,uuid; h=bytearray(hashlib.md5(("OfflinePlayer:"+sys.argv[1]).encode()).digest()); h[6]=h[6]&0x0f|0x30; h[8]=h[8]&0x3f|0x80; print(uuid.UUID(bytes=bytes(h)))' "$USERNAME")

echo "vanilla-client: $USERNAME -> $SERVER, scenario $(basename "$SCENARIO_FILE"), ${WIDTH}x${HEIGHT}, log $GAME/logs/latest.log"
cd "$GAME"
# Wayland gives the window a HiDPI framebuffer (desktop scale x size); through XWayland the
# framebuffer is exactly WIDTHxHEIGHT, which the screenshots need.
export SDL_VIDEODRIVER=${HASTE_VANILLA_VIDEO:-x11}
export SDL_VIDEO_X11_SCALING_FACTOR=1
set +e
timeout --kill-after=10 "$TIMEOUT" "$JAVA" -Xmx3G \
  -Djava.library.path="$NATIVES/java" \
  -Djna.tmpdir="$NATIVES/jna" \
  -Dorg.lwjgl.system.SharedLibraryExtractPath="$NATIVES/lwjgl" \
  -Dio.netty.native.workdir="$NATIVES/netty" \
  --enable-native-access=ALL-UNNAMED \
  -Dfabric.development=false \
  -DFabricMcEmu=net.minecraft.client.main.Main \
  -Dhaste.scenario="$SCENARIO_FILE" \
  -Dhaste.oracle.out="$OUT" \
  -cp "$CP" net.fabricmc.loader.impl.launch.knot.KnotClient \
  --username "$USERNAME" --uuid "$UUID" --accessToken 0 --userType legacy \
  --version 26.3 --versionType release \
  --gameDir "$GAME" --assetsDir "$CACHE/assets" --assetIndex "$ASSET_INDEX" \
  --width "$WIDTH" --height "$HEIGHT" \
  --quickPlayMultiplayer "$SERVER" \
  > "$GAME/client-stdout.log" 2>&1
STATUS=$?
set -e
if [ $STATUS -eq 124 ] || [ $STATUS -eq 137 ]; then
  echo "vanilla-client: timed out after ${TIMEOUT}s (see $GAME/logs/latest.log)" >&2
fi
grep -h "haste-recorder" "$GAME/logs/latest.log" 2>/dev/null | tail -40 || true
exit $STATUS
