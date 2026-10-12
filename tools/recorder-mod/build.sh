#!/usr/bin/env bash
# Compiles the haste-recorder Fabric mod (server side) against the unobfuscated 26.3 server jar +
# Fabric Loader + sponge-mixin + the server's libraries, into build/haste-recorder.jar.
# No Gradle/Loom: 26.3 ships Mojang names and no intermediary, so plain javac works (see the spike).
# Usage: tools/recorder-mod/build.sh   (from anywhere; resolves the repo root itself)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

# Repo root (two levels up from tools/recorder-mod), then the shared cache.
ROOT="$(cd "$HERE/../.." && pwd)"
CACHE="${HASTE_CACHE:-$ROOT/cache}"
JDK="${HASTE_JAVA:-$CACHE/jdk/bin}"
if [ ! -x "$JDK/java" ]; then
  # fall back to a system java if the cache JDK is absent
  JDK="$(dirname "$(command -v java)")"
fi
export JAVA_HOME="${JAVA_HOME:-$(cd "$JDK/.." && pwd)}"

SERVER_JAR="$CACHE/server-spike/versions/26.3/server-26.3.jar"
CLIENT_JAR="$CACHE/versions/26.3/client.jar"
[ -f "$SERVER_JAR" ] || { echo "missing $SERVER_JAR (run from the repo with cache/ present)" >&2; exit 1; }
[ -f "$CLIENT_JAR" ] || { echo "missing $CLIENT_JAR (client classes needed for the recorder)" >&2; exit 1; }

LIBS="$CACHE/server-spike/libraries"
MIXIN=$(ls "$LIBS"/net/fabricmc/sponge-mixin/*/sponge-mixin-*.jar | head -1)
LOADER=$(ls "$LIBS"/net/fabricmc/fabric-loader/*/fabric-loader-*.jar | head -1)
SLF4J=$(find "$LIBS" -name 'slf4j-api-*.jar' | head -1)
GSON=$(find "$LIBS" -name 'gson-*.jar' | grep -v sources | head -1)
AUTHLIB=$(find "$LIBS" -name 'authlib-*.jar' | grep -v sources | head -1)
BRIGADIER=$(find "$LIBS" -name 'brigadier-*.jar' | grep -v sources | head -1)
# Fabric API (client lifecycle events, client commands) from the server-spike mods; its
# modules are nested jars, which javac cannot read, so unpack the two we need.
FABRIC_API=$(ls "$CACHE/server-spike/mods/fabric-api"*.jar | head -1)
FABRIC_CP=""
if [ -n "$FABRIC_API" ]; then
  NESTED="$HERE/build/fabric-api-nested"
  rm -rf "$NESTED" && mkdir -p "$NESTED"
  for mod in fabric-api-base fabric-command-api-v2 fabric-lifecycle-events-v1; do
    nested_jar=$(unzip -l "$FABRIC_API" "META-INF/jars/${mod}-*.jar" | awk '{print $4}' | grep -v '^$' | tail -1)
    if [ -n "$nested_jar" ]; then
      unzip -o -q "$FABRIC_API" "$nested_jar" -d "$NESTED" || true
      FABRIC_CP="$FABRIC_CP$(find "$NESTED" -name '*.jar' | tr '\n' ':')"
    fi
  done
fi
ALLLIBS=$(find "$LIBS" -name '*.jar' ! -name '*sources*' | tr '\n' ':')

CP="$SERVER_JAR:$CLIENT_JAR:$MIXIN:$LOADER:$FABRIC_CP$SLF4J:$GSON:$AUTHLIB:$BRIGADIER:$ALLLIBS"

rm -rf build/classes build/haste-recorder.jar
mkdir -p build/classes
"$JDK/javac" -d build/classes -cp "$CP" \
  $(find src/main/java -name '*.java') \
  -implicit:class
cp -r src/main/resources/. build/classes/
(cd build/classes && "$JDK/jar" cf ../haste-recorder.jar .)
ls -la build/haste-recorder.jar
