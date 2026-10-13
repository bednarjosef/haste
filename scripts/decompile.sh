#!/usr/bin/env bash
# Decompile the player's 26.3 client jar into the gitignored cache for reading
# (AGENTS.md rule 5: read it for exact numbers, orders and logic; write Haste's
# code yourself; never commit it or paste it into the tree).
#
#   scripts/decompile.sh            # -> $HASTE_CACHE/source/26.3/client/net/minecraft/...
#
# Uses Vineflower (Apache-2.0) from Maven Central, verified by its published SHA-1.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
VF_VERSION=1.12.0
VF_SHA1=85570609a0a5941a7d2918b6260b209de810f66f
VF="$HASTE_CACHE/tools/vineflower-$VF_VERSION.jar"
JAR="${HASTE_CLIENT_JAR:-$HASTE_CACHE/mojang-spike/client-26.3.jar}"
OUT="$HASTE_CACHE/source/26.3/client"

if [ ! -f "$VF" ]; then
  mkdir -p "$(dirname "$VF")"
  curl -sfL -o "$VF.part" \
    "https://repo1.maven.org/maven2/org/vineflower/vineflower/$VF_VERSION/vineflower-$VF_VERSION.jar"
  echo "$VF_SHA1  $VF.part" | sha1sum -c --quiet
  mv "$VF.part" "$VF"
fi
[ -f "$JAR" ] || { echo "no client jar at $JAR (run haste setup)" >&2; exit 1; }
rm -rf "$OUT" && mkdir -p "$OUT"
nice -n 19 java -Xmx4g -jar "$VF" -dgs=1 -rsy=1 -log=WARN -thr=4 \
  --only=net/minecraft --only=com/mojang "$JAR" "$OUT"
# Vineflower copies the jar's resources too; keep only source (assets come from the jar at runtime).
find "$OUT" -mindepth 1 -maxdepth 1 ! -name net ! -name com -exec rm -rf {} +
echo "$(find "$OUT" -name '*.java' | wc -l) classes in $OUT"
