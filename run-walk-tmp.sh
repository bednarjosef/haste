#!/bin/sh
# Walk-script headless check for the own-block-sounds branch.
export HASTE_WALK_SCRIPT=crates/haste-client/walks/block-sounds.txt
cargo run -p haste-client -j 4 -q --bin haste -- --headless --server 127.0.0.1:25691 --seconds 60
