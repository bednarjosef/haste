#!/usr/bin/env bash
# Starts the 26.3 vanilla dedicated server with Fabric Loader + Fabric API: offline mode,
# bound to 127.0.0.1:25565, headless (nogui). Installs the test mod jar if it has been built.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
export JAVA_HOME="$ROOT/cache/jdk" PATH="$ROOT/cache/jdk/bin:$PATH"
cd cache/server
echo "eula=true" > eula.txt
cat > server.properties <<'PROPS'
online-mode=false
server-ip=127.0.0.1
server-port=25565
level-type=minecraft\:flat
gamemode=creative
spawn-protection=0
max-players=5
motd=haste-spike
sync-chunk-writes=false
pause-when-empty-seconds=-1
PROPS
mkdir -p mods
cp -f ../dl/fabric-api-*.jar mods/
[ -f "$ROOT/mod/build/haste-test-mod.jar" ] && cp -f "$ROOT/mod/build/haste-test-mod.jar" mods/
exec java -Xms1G -Xmx2G -jar fabric-server-launch.jar nogui "$@"
