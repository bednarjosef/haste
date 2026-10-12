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
[ -f "$SERVER_JAR" ] || { echo "missing $SERVER_JAR (run from the repo with cache/ present)" >&2; exit 1; }

LIBS="$CACHE/server-spike/libraries"
MIXIN=$(ls "$LIBS"/net/fabricmc/sponge-mixin/*/sponge-mixin-*.jar | head -1)
LOADER=$(ls "$LIBS"/net/fabricmc/fabric-loader/*/fabric-loader-*.jar | head -1)
SLF4J=$(find "$LIBS" -name 'slf4j-api-*.jar' | head -1)
GSON=$(find "$LIBS" -name 'gson-*.jar' | grep -v sources | head -1)
AUTHLIB=$(find "$LIBS" -name 'authlib-*.jar' | grep -v sources | head -1)
BRIGADIER=$(find "$LIBS" -name 'brigadier-*.jar' | grep -v sources | head -1)
ALLLIBS=$(find "$LIBS" -name '*.jar' ! -name '*sources*' | tr '\n' ':')

CP="$SERVER_JAR:$MIXIN:$LOADER:$SLF4J:$GSON:$AUTHLIB:$BRIGADIER:$ALLLIBS"

rm -rf build/classes build/haste-recorder.jar
mkdir -p build/classes
"$JDK/javac" -d build/classes -cp "$CP" \
  $(find src/main/java -name '*.java') \
  -implicit:class
cp -r src/main/resources/. build/classes/
(cd build/classes && "$JDK/jar" cf ../haste-recorder.jar .)
ls -la build/haste-recorder.jar
