#!/usr/bin/env bash
# Compiles the test Fabric mod (against the unobfuscated 26.3 client jar + Mixin + Fabric Loader) into
# mod/build/haste-test-mod.jar, and the ASM bridge into landmark/build/bridge/. No Gradle: see REPORT.md.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
export JAVA_HOME="$ROOT/cache/jdk" PATH="$ROOT/cache/jdk/bin:$PATH"
LIBS=cache/server/libraries
MIXIN=$(ls $LIBS/net/fabricmc/sponge-mixin/*/sponge-mixin-*.jar | head -1)
LOADER=$(ls $LIBS/net/fabricmc/fabric-loader/*/fabric-loader-*.jar | head -1)
SLF4J=$(find $LIBS -name 'slf4j-api-*.jar' | head -1)
ALLLIBS=$(find $LIBS -name '*.jar' | tr '\n' ':')  # vanilla classes reference fastutil, jspecify, joml, ...
ASM=$(ls cache/dl/asm-*.jar | tr '\n' ':')

echo "== mod"
rm -rf mod/build && mkdir -p mod/build/classes
javac -d mod/build/classes -cp "cache/dl/client-26.3.jar:$MIXIN:$LOADER:$SLF4J:$ALLLIBS" $(find mod/src/main/java -name '*.java')
cp -r mod/src/main/resources/. mod/build/classes/
(cd mod/build/classes && jar cf ../haste-test-mod.jar .)
ls -la mod/build/haste-test-mod.jar

echo "== bridge"
rm -rf landmark/build && mkdir -p landmark/build/bridge
javac -d landmark/build/bridge -cp "$ASM" landmark/java/haste/bridge/*.java landmark/java/haste/rt/*.java landmark/java/haste/proxy/*.java

echo "== proxies (pure + mirror)"
sed -e 's|^native \(net/minecraft/world/entity/Entity#getX()D\)$|getter \1=haste$x:D|' \
    -e 's|^native \(net/minecraft/world/item/ItemStack#getCount()I\)$|getter \1=count|' landmark/proxies.conf > landmark/build/proxies-mirror.conf
for mode in pure mirror; do
  conf=landmark/proxies.conf; [ $mode = mirror ] && conf=landmark/build/proxies-mirror.conf
  java -cp "landmark/build/bridge:$ASM" haste.proxy.Gen cache/dl/client-26.3.jar mod/build/haste-test-mod.jar $conf landmark/build/proxies-$mode
done
echo "MIXIN_JAR=$MIXIN" > landmark/build/paths.env
echo "ASM_CP=$ASM" >> landmark/build/paths.env
echo "== ok"
