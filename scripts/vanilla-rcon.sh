#!/usr/bin/env bash
# Sends commands to the reference server (scripts/vanilla-server.sh) over RCON.
# usage: scripts/vanilla-rcon.sh [--port P] setup            fixed world state (see below)
#        scripts/vanilla-rcon.sh [--port P] <command> ...    one command per argument
# The RCON port is the server port + 1000 (scripts/test-server.sh), default 26620.
set -euo pipefail
PORT=26620
if [ "${1:-}" = --port ]; then PORT=$(( $2 + 1000 )); shift 2; fi
[ $# -gt 0 ] || { echo "usage: vanilla-rcon.sh [--port P] setup | <command>..." >&2; exit 2; }
if [ "$1" = setup ]; then
  # Reproducible pictures: frozen noon, clear weather, nothing spawning or moving.
  set -- "gamerule advance_time false" "gamerule advance_weather false" "gamerule spawn_mobs false" \
    "gamerule spawn_monsters false" "gamerule spawn_patrols false" "gamerule spawn_phantoms false" \
    "gamerule spawn_wandering_traders false" "gamerule random_tick_speed 0" \
    "gamerule respawn_radius 0" "gamerule send_command_feedback false" \
    "gamerule show_advancement_messages false" "time set 6000" "weather clear"
fi
exec python3 -I - "$PORT" "$@" <<'PY'
import socket, struct, sys
port, cmds = int(sys.argv[1]), sys.argv[2:]
s = socket.create_connection(("127.0.0.1", port), timeout=10)
def send(rid, kind, body):
    data = struct.pack("<ii", rid, kind) + body.encode() + b"\0\0"
    s.sendall(struct.pack("<i", len(data)) + data)
def recv():
    n = struct.unpack("<i", s.recv(4, socket.MSG_WAITALL))[0]
    data = s.recv(n, socket.MSG_WAITALL)
    rid, _ = struct.unpack("<ii", data[:8])
    return rid, data[8:-2].decode(errors="replace")
send(1, 3, "haste-local")
if recv()[0] == -1:
    sys.exit("rcon: authentication failed")
for i, c in enumerate(cmds, 2):
    send(i, 2, c)
    out = recv()[1]
    print(f"> {c}" + (f"\n{out}" if out else ""))
PY
