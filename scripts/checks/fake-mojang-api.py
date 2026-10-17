#!/usr/bin/env python3
"""A tiny stand-in for Mojang's session and services APIs, for testing online mode locally.

usage: fake-mojang-api.py <state-dir> <port> [player-name]

Binds to 127.0.0.1 only. On start it generates, with the openssl CLI, a "services" signing key
(what Mojang signs player certificates with) and one player key pair, writes
<state-dir>/account.json (the fake access token and profile for Haste's HASTE_TEST_ACCOUNT) and
logs every request to <state-dir>/requests.log.

Endpoints (the vanilla server finds them through authlib's discovery document, started with
-Dminecraft.api.discovery.host=http://127.0.0.1:<port>/discovery; Haste uses the same paths as
Mojang's real hosts under HASTE_SESSION_HOST / HASTE_SERVICES_HOST):
  GET  /discovery                               authlib's service discovery
  POST /session/minecraft/join                  client: joining a server (204)
  GET  /session/minecraft/hasJoined             server: did <username> join <serverId>?
  GET  /session/minecraft/profile/<uuid>        profile by id
  GET  /minecraft/profile/lookup/name/<name>    profile by name
  POST /profiles/minecraft                      profiles by names
  POST /player/certificates                     the player's chat key pair, signed
  GET  /publickeys                              the services public keys (key signature check)
Nothing here talks to any other host.
"""

import base64
import json
import os
import secrets
import subprocess
import sys
import threading
import time
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse


def openssl(*args, data=None):
    return subprocess.run(["openssl", *args], input=data, capture_output=True, check=True).stdout


def gen_key(path, bits):
    openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", f"rsa_keygen_bits:{bits}", "-out", path)


def b64(data):
    return base64.b64encode(data).decode()


def pem(label, der):
    body = base64.encodebytes(der).decode().replace("\n", "")
    lines = [body[i:i + 76] for i in range(0, len(body), 76)]
    return f"-----BEGIN {label}-----\n" + "\n".join(lines) + f"\n-----END {label}-----\n"


def iso_millis(ms):
    t = datetime.fromtimestamp(ms / 1000, tz=timezone.utc)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"


class State:
    def __init__(self, directory, name):
        self.dir = directory
        os.makedirs(directory, exist_ok=True)
        services = os.path.join(directory, "services.pem")
        player = os.path.join(directory, "player.pem")
        gen_key(services, 4096)
        gen_key(player, 2048)
        self.services_pub = openssl("pkey", "-in", services, "-pubout", "-outform", "DER")
        # PKCS#8 and X.509 DER, inside the "RSA ... KEY" armour Mojang uses
        # (vanilla Crypt.stringToPemRsaPrivateKey / stringToRsaPublicKey).
        self.player_priv = openssl("pkey", "-in", player, "-outform", "DER")
        self.player_pub = openssl("pkey", "-in", player, "-pubout", "-outform", "DER")
        self.uuid = uuid.uuid4()
        self.name = name
        self.token = secrets.token_hex(32)
        now = int(time.time() * 1000)
        # Odd milliseconds on purpose: the signed payload carries the expiry in milliseconds.
        self.expires_ms = now + 48 * 3600 * 1000 + 123
        self.refreshed_ms = now + 40 * 3600 * 1000 + 456
        # ProfilePublicKey.Data.signedPayload: uuid (msb, lsb), expiry millis, key DER; the
        # services key signs it with SHA1withRSA (authlib ServicesKeyInfo).
        payload = self.uuid.bytes + self.expires_ms.to_bytes(8, "big") + self.player_pub
        self.signature_v2 = openssl("dgst", "-sha1", "-sign", services, data=payload)
        self.joins = {}  # serverId -> (name, uuid)
        self.lock = threading.Lock()
        with open(os.path.join(directory, "account.json"), "w") as f:
            json.dump({"access_token": self.token, "uuid": str(self.uuid), "name": name}, f)
        self.log = open(os.path.join(directory, "requests.log"), "a", buffering=1)

    def certificates(self):
        return {
            "keyPair": {
                "privateKey": pem("RSA PRIVATE KEY", self.player_priv),
                "publicKey": pem("RSA PUBLIC KEY", self.player_pub),
            },
            # v1 is deliberately not a valid signature: clients must use v2.
            "publicKeySignature": b64(secrets.token_bytes(512)),
            "publicKeySignatureV2": b64(self.signature_v2),
            "expiresAt": iso_millis(self.expires_ms),
            "refreshedAfter": iso_millis(self.refreshed_ms),
        }

    def profile(self):
        return {"id": self.uuid.hex, "name": self.name, "properties": []}


def make_handler(state, base):
    def endpoint(path):
        return {"uri": base + path, "validUris": []}

    discovery = {
        "environment": "local",
        "product": "minecraft",
        "discovery": {
            "product": "minecraft",
            "authentication": {"endpoints": {"getPublicKeys": endpoint("/publickeys")}},
            "session": {"endpoints": {
                "join": endpoint("/session/minecraft/join"),
                "verify": endpoint("/session/minecraft/hasJoined"),
                "getProfileById": endpoint("/session/minecraft/profile/{profileId}"),
            }},
            "player": {"endpoints": {"getCertificates": endpoint("/player/certificates")}},
            "profiles": {"endpoints": {
                "getByName": endpoint("/minecraft/profile/lookup/name/{name}"),
                "getManyByName": endpoint("/profiles/minecraft"),
            }},
            "telemetry": {"endpoints": {}},
        },
    }

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            state.log.write(f"{self.log_date_time_string()} {fmt % args}\n")

        def reply(self, code, body=None):
            data = b"" if body is None else json.dumps(body).encode()
            self.send_response(code)
            if body is not None:
                self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def body(self):
            n = int(self.headers.get("Content-Length") or 0)
            return self.rfile.read(n) if n else b""

        def authorised(self):
            return self.headers.get("Authorization") == f"Bearer {state.token}"

        def do_GET(self):
            url = urlparse(self.path)
            q = {k: v[0] for k, v in parse_qs(url.query).items()}
            if url.path == "/discovery":
                return self.reply(200, discovery)
            if url.path == "/publickeys":
                key = {"publicKey": b64(state.services_pub)}
                return self.reply(200, {"profilePropertyKeys": [key], "playerCertificateKeys": [key]})
            if url.path == "/session/minecraft/hasJoined":
                with state.lock:
                    joined = state.joins.get(q.get("serverId"))
                ok = joined is not None and joined[0] == q.get("username")
                state.log.write(f"hasJoined username={q.get('username')} serverId={q.get('serverId')} -> {ok}\n")
                return self.reply(200, state.profile()) if ok else self.reply(204)
            if url.path.startswith("/session/minecraft/profile/"):
                wanted = url.path.rsplit("/", 1)[1].replace("-", "")
                return self.reply(200, state.profile()) if wanted == state.uuid.hex else self.reply(204)
            if url.path.startswith("/minecraft/profile/lookup/name/"):
                wanted = url.path.rsplit("/", 1)[1]
                if wanted.lower() == state.name.lower():
                    return self.reply(200, {"id": state.uuid.hex, "name": state.name})
                return self.reply(404, {"errorMessage": "Couldn't find any profile with name " + wanted})
            return self.reply(404, {"error": "Not Found", "path": url.path})

        def do_POST(self):
            url = urlparse(self.path)
            raw = self.body()
            if url.path == "/session/minecraft/join":
                req = json.loads(raw or b"{}")
                if req.get("accessToken") != state.token or req.get("selectedProfile", "").replace("-", "") != state.uuid.hex:
                    state.log.write("join refused: bad token or profile\n")
                    return self.reply(403, {"error": "ForbiddenOperationException", "errorMessage": "Invalid token"})
                with state.lock:
                    state.joins[req.get("serverId")] = (state.name, state.uuid)
                state.log.write(f"join serverId={req.get('serverId')} profile={state.uuid.hex}\n")
                return self.reply(204)
            if url.path == "/player/certificates":
                if not self.authorised():
                    return self.reply(401, {"error": "Unauthorized"})
                return self.reply(200, state.certificates())
            if url.path == "/profiles/minecraft":
                names = json.loads(raw or b"[]")
                hits = [{"id": state.uuid.hex, "name": state.name} for n in names if n.lower() == state.name.lower()]
                return self.reply(200, hits)
            return self.reply(404, {"error": "Not Found", "path": url.path})

    return Handler


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    directory, port = sys.argv[1], int(sys.argv[2])
    name = sys.argv[3] if len(sys.argv) > 3 else "online-local"
    state = State(directory, name)
    base = f"http://127.0.0.1:{port}"
    server = ThreadingHTTPServer(("127.0.0.1", port), make_handler(state, base))
    print(f"fake Mojang API on {base} for {name} ({state.uuid})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
