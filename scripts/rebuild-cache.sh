#!/usr/bin/env bash
# Rebuilds the shared derived-data cache from scratch (everything in it is reproducible).
# Real location: ../haste-cache next to the repo (override with HASTE_CACHE_REAL); ./cache is a symlink to it.
set -euo pipefail
H="$(cd "$(dirname "$0")/.." && pwd)"; C=${HASTE_CACHE_REAL:-$(dirname "$H")/haste-cache}
SP=${HASTE_SPIKE:-$(dirname "$H")/haste-spike}/cache   # spike's JDK, Mojang jars and Fabric server (until haste-setup replaces them)
mkdir -p "$C"; [ -e "$H/cache" ] && [ ! -L "$H/cache" ] && { echo "$H/cache is a real directory; move it aside first"; exit 1; }
ln -sfn "$C" "$H/cache"
mkdir -p "$C/versions/26.3" "$C/datagen/26.3" "$C/oracle/26.3"
ln -sfn "$SP/jdk" "$C/jdk"; ln -sfn "$SP/dl" "$C/mojang-spike"; ln -sfn "$SP/server" "$C/server-spike"
ln -sfn "$SP/dl/client-26.3.jar" "$C/versions/26.3/client.jar"; ln -sfn "$SP/dl/server-26.3.jar" "$C/versions/26.3/server.jar"
if [ ! -f "$C/datagen/26.3/reports/blocks.json" ]; then
  echo "== data generator"; mkdir -p "$C/datagen/26.3/work" && (cd "$C/datagen/26.3/work" && "$C/jdk/bin/java" -DbundlerMainClass=net.minecraft.data.Main -jar "$C/versions/26.3/server.jar" --reports --all >/dev/null 2>&1) && mv "$C/datagen/26.3/work/generated/reports" "$C/datagen/26.3/reports" && rm -rf "$C/datagen/26.3/work"
fi
[ -d "$C/pomme" ] || git clone -q --depth 1 https://github.com/PommeMC/Client "$C/pomme"
[ -d "$C/pumpkin" ] || git clone -q --depth 1 https://github.com/Pumpkin-MC/Pumpkin "$C/pumpkin"
if [ ! -f "$C/oracle/26.3/block_shapes.json" ]; then echo "== oracle dumps"; (cd "$H" && tools/recorder-mod/dump-server-oracles.sh); fi
echo "== cache rebuilt at $C"; ls "$C"
