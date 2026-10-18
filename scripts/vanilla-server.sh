#!/usr/bin/env bash
# The reference server both clients (vanilla and Haste) are compared on: Mojang's plain 26.3
# server, offline mode, a fixed-seed normal world, peaceful, no time/weather change, RCON on
# for scripted commands (scripts/vanilla-rcon.sh), and the reference players opped.
# usage: scripts/vanilla-server.sh [port] [seconds]     (default port 25620, 3600 s)
# env:   HASTE_REF_SEED (default below; changing it regenerates the world)
#        HASTE_REF_PLAYERS (default "HasteRef haste": opped offline names)
#        HASTE_REF_VIEW_DISTANCE (default 10; the server caps every client's render distance)
# Runs in the foreground (start it with & or in another terminal); it prints "Done" when ready.
# The world settings (gamerules, time) are applied by scripts/vanilla-rcon.sh setup once it is up.
set -euo pipefail
PORT=${1:-25620}; SECS=${2:-3600}
SEED=${HASTE_REF_SEED:-20263}
PLAYERS=${HASTE_REF_PLAYERS:-HasteRef haste}
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
NAME=vanilla-reference
DIR="$CACHE/servers/$NAME"
if [ ! -d "$DIR" ]; then
  mkdir -p "$DIR"
  (cd "$CACHE/server-spike" && tar --exclude=./logs --exclude=./world -cf - .) | tar -xf - -C "$DIR"
fi
# A different seed than the existing world means a new world.
if [ "$(cat "$DIR/seed" 2>/dev/null)" != "$SEED" ]; then rm -rf "$DIR/world"; fi
echo "$SEED" > "$DIR/seed"
sed -i -e "s/^level-seed=.*/level-seed=$SEED/" -e "s/^difficulty=.*/difficulty=peaceful/" \
  -e "s/^enable-rcon=.*/enable-rcon=true/" -e "s/^rcon.password=.*/rcon.password=haste-local/" \
  -e "s/^gamemode=.*/gamemode=creative/" -e "s/^view-distance=.*/view-distance=${HASTE_REF_VIEW_DISTANCE:-10}/" \
  -e "s/^generate-structures=.*/generate-structures=true/" -e "s/^max-players=.*/max-players=10/" \
  -e "s/^enforce-secure-profile=.*/enforce-secure-profile=false/" \
  "$DIR/server.properties"
# ops.json: offline UUIDs (v3 of "OfflinePlayer:<name>"), permission level 4
python3 -I - "$DIR/ops.json" $PLAYERS <<'PY'
import hashlib, json, sys, uuid
ops = []
for name in sys.argv[2:]:
    h = bytearray(hashlib.md5(("OfflinePlayer:" + name).encode()).digest())
    h[6] = h[6] & 0x0F | 0x30; h[8] = h[8] & 0x3F | 0x80
    ops.append({"uuid": str(uuid.UUID(bytes=bytes(h))), "name": name, "level": 4, "bypassesPlayerLimit": False})
json.dump(ops, open(sys.argv[1], "w"), indent=2)
PY
HASTE_SERVER=vanilla exec "$ROOT/scripts/test-server.sh" "$NAME" "$PORT" "$SECS" minecraft:normal
