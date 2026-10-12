#!/usr/bin/env bash
# Starts the local Fabric 26.3 test server (offline mode, 127.0.0.1:25565) from
# the shared cache for M1 headless runs. Usage: local-server.sh [seconds]
# Stop it with: kill "$(pgrep -f 'java.*fabric-server-launch.jar nogui')".
# The server whitelist is on; offline user `haste` has UUID
# 78280295-106a-3287-91da-97b4b0c31f56 (v3 of "OfflinePlayer:haste").
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
cd "$CACHE/server-spike"
exec timeout "${1:-3600}" "$CACHE/jdk/bin/java" -Xmx2G -jar fabric-server-launch.jar nogui
