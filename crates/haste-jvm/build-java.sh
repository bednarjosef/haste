#!/usr/bin/env bash
# Compiles haste-jvm's Java sources with plain javac (no Gradle) into build/:
#   build/boot  haste.loader, haste.proxy (needs ASM), haste.rt   -> the JVM's application class path
#   build/game  haste.gui (compiled AGAINST the client jar, never run with it on the class path;
#               served at runtime by HasteClassLoader next to the generated proxies)
# usage: build-java.sh <java_home> <client.jar> <out dir> <asm jars ':'-separated> <library jars ':'-separated>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
JAVA_HOME="$1"; CLIENT="$2"; OUT="$3"; ASM="$4"; LIBS="$5"
[ -n "$OUT" ] && [ -d "$JAVA_HOME" ] || { echo "usage: $0 <java_home> <client.jar> <out> <asm cp> <libs cp>" >&2; exit 2; }
JAVAC="$JAVA_HOME/bin/javac"
OPTS=(-nowarn -g -encoding UTF-8 --release 25 -Xlint:none)
mkdir -p "$OUT"
B="$OUT/boot.tmp$$"; G="$OUT/game.tmp$$"
rm -rf "$B" "$G"; mkdir -p "$B" "$G"
"$JAVAC" "${OPTS[@]}" -d "$B" -cp "$ASM" \
  $(find "$HERE/java/haste/loader" "$HERE/java/haste/proxy" "$HERE/java/haste/rt" -name '*.java')
"$JAVAC" "${OPTS[@]}" -d "$G" -cp "$B:$CLIENT:$LIBS" \
  $(find "$HERE/java/haste/gui" -name '*.java')
rm -rf "$OUT/boot" "$OUT/game"
mv "$B" "$OUT/boot"
mv "$G" "$OUT/game"
