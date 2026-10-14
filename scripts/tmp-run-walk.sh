#!/bin/sh
# Temporary runner: headless join with the block-sounds walk script.
export HASTE_WALK_SCRIPT=crates/haste-client/walks/block-sounds.txt
exec ./target/debug/haste --headless --server 127.0.0.1:25691 --seconds 60
