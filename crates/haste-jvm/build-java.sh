#!/usr/bin/env bash
# Compiles haste-jvm's Java sources with plain javac (no Gradle) into <out>:
#   boot  haste.loader, haste.proxy (needs ASM), haste.rt   -> the JVM's application class path
#   game  haste.gui (compiled AGAINST the client jar, never run with it on the class path;
#         served at runtime by HasteClassLoader next to the generated proxies)
# usage: build-java.sh <boot|game> <java_home> <client.jar> <out dir> <asm jars ':'-separated> <library jars ':'-separated>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WHAT="$1"; JAVA_HOME="$2"; CLIENT="$3"; OUT="$4"; ASM="$5"; LIBS="$6"
[ -n "$OUT" ] && [ -d "$JAVA_HOME" ] || { echo "usage: $0 <boot|game> <java_home> <client.jar> <out> <asm cp> <libs cp>" >&2; exit 2; }
JAVAC=("$JAVA_HOME/bin/javac" -J-XX:TieredStopAtLevel=1 -J-XX:+UseSerialGC -J-Xshare:auto)
OPTS=(-nowarn -g -encoding UTF-8 --release 25 -Xlint:none -implicit:none)
mkdir -p "$OUT"
TMP="$OUT/$WHAT.tmp$$"
rm -rf "$TMP"; mkdir -p "$TMP"
case "$WHAT" in
  boot) "${JAVAC[@]}" "${OPTS[@]}" -d "$TMP" -cp "$ASM" \
          $(find "$HERE/java/haste/loader" "$HERE/java/haste/proxy" "$HERE/java/haste/rt" -name '*.java') ;;
  game) "${JAVAC[@]}" "${OPTS[@]}" -d "$TMP" -cp "$OUT/boot:$CLIENT:$LIBS" \
          $(find "$HERE/java/haste/gui" -name '*.java') ;;
  *) echo "unknown target $WHAT" >&2; exit 2 ;;
esac
rm -rf "$OUT/$WHAT"
mv "$TMP" "$OUT/$WHAT"
