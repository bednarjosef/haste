#!/usr/bin/env bash
# Starts a private copy of the local Fabric 26.3 test server (offline mode, flat creative world)
# so several worktrees can test against their own server at once.
# usage: [HASTE_SERVER=vanilla] scripts/test-server.sh <name> <port> [seconds] [level-type]
#   HASTE_SERVER=vanilla runs Mojang's plain server.jar instead of Fabric (the purer oracle for
#   protocol compatibility; Fabric adds its own channels and registry sync)
#   <name>  copy lives in $HASTE_CACHE/servers/<name> (created from cache/server-spike on first use)
#   <port>  server-port; each workstream brief names its own
#   level-type defaults to minecraft:flat; pass minecraft:normal for terrain (delete world/ first)
#   HASTE_SERVER_HEAP sets the Java heap (default 2G: a shared server with many test players
#   and summoned entities ran out of 1500M after ~21 hours)
# Stop: kill "$(pgrep -f "servers/<name>.*fabric-server-launch")" or let the timeout end it.
# Logs: $HASTE_CACHE/servers/<name>/logs/latest.log ("moved wrongly", "moved too quickly",
# kicks and disconnect reasons are the server's verdict on our client).
set -euo pipefail
NAME=${1:?name}; PORT=${2:?port}; SECS=${3:-3600}; LEVEL=${4:-minecraft:flat}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
DIR="$CACHE/servers/$NAME"
if [ ! -d "$DIR" ]; then
  mkdir -p "$DIR"
  (cd "$CACHE/server-spike" && tar --exclude=./logs --exclude=./world -cf - .) | tar -xf - -C "$DIR"
fi
sed -i -e "s/^server-port=.*/server-port=$PORT/" -e "s/^query.port=.*/query.port=$PORT/" \
  -e "s/^level-type=.*/level-type=${LEVEL//:/\\\\:}/" -e "s/^rcon.port=.*/rcon.port=$((PORT + 1000))/" -e "s/^white-list=.*/white-list=false/" \
  "$DIR/server.properties"
cd "$DIR"
JAR=fabric-server-launch.jar
[ "${HASTE_SERVER:-fabric}" = vanilla ] && JAR=server.jar
exec timeout "$SECS" "$CACHE/jdk/bin/java" "-Xmx${HASTE_SERVER_HEAP:-2G}" -jar "$JAR" nogui
