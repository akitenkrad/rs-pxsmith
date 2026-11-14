#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$project_dir/../.." && pwd)
pxsmith=${PXSMITH:-"$repo_dir/target/debug/pxsmith"}
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/pxsmith-godot-ui.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

if [ -z "${PXSMITH:-}" ]; then
    cargo build --manifest-path "$repo_dir/Cargo.toml" -p pxsmith
elif [ ! -x "$pxsmith" ]; then
    echo "PXSMITH is not executable: $pxsmith" >&2
    exit 1
fi

misaki_font="$work_dir/misaki.pxfont.toml"
gpos_font="$work_dir/gpos.pxfont.toml"
misaki_text=$(sed -n '1,2p' "$project_dir/misaki_chars.txt")

"$pxsmith" font import \
    "$repo_dir/testdata/fonts/misaki/misaki_gothic.bdf" \
    "$misaki_font" \
    --chars-file "$project_dir/misaki_chars.txt"
"$pxsmith" font bmfont \
    "$misaki_font" \
    "$project_dir/misaki.fnt" \
    --chars-file "$project_dir/misaki_chars.txt" \
    --page-size 256
"$pxsmith" font render \
    "$misaki_font" \
    "$misaki_text" \
    "$project_dir/misaki_expected.png" \
    --palette "$project_dir/white.hex" \
    --ink 0

"$pxsmith" font import \
    "$repo_dir/testdata/fonts/synthetic/pxsynth-gpos.ttf" \
    "$gpos_font" \
    --chars-file "$project_dir/gpos_chars.txt" \
    --size 8
"$pxsmith" font bmfont \
    "$gpos_font" \
    "$project_dir/gpos.fnt" \
    --chars-file "$project_dir/gpos_chars.txt" \
    --page-size 256
"$pxsmith" font render \
    "$gpos_font" \
    AV \
    "$project_dir/gpos_expected.png" \
    --palette "$project_dir/white.hex" \
    --ink 0
