#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$project_dir/../.." && pwd)
pxsmith=${PXSMITH:-"$repo_dir/target/debug/pxsmith"}

if [ ! -x "$pxsmith" ]; then
    cargo build --manifest-path "$repo_dir/Cargo.toml" -p pxsmith
fi

"$pxsmith" sheet pack "$project_dir/sheet.png" \
    --input "$repo_dir/examples/sprites/hero_body.px.toml" \
    --meta "$project_dir/sheet.sheet.json" \
    --palette-swap \
    --variant "$repo_dir/palettes/sample.hex=$project_dir/red.hex" \
    --variant "$repo_dir/palettes/sample.hex=$project_dir/blue.hex"
