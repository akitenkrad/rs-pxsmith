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
tile_window="$work_dir/window-tile.px.toml"
tile_fit_window="$work_dir/window-tile-fit.px.toml"
sheet_stretch_window="$work_dir/window_stretch.px.toml"
sheet_tile_window="$work_dir/window_tile.px.toml"
sheet_tile_fit_window="$work_dir/window_tile_fit.px.toml"
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

cp "$project_dir/window.hex" "$work_dir/window.hex"
sed 's/bottom = 4 }/bottom = 4, stretch = "tile" }/' \
    "$project_dir/window.px.toml" > "$tile_window"
sed 's/bottom = 4 }/bottom = 4, stretch = "tile_fit" }/' \
    "$project_dir/window.px.toml" > "$tile_fit_window"
cp "$project_dir/window.px.toml" "$sheet_stretch_window"
sed 's/bottom = 4 }/bottom = 4, stretch = "tile" }/' \
    "$project_dir/window.px.toml" > "$sheet_tile_window"
sed 's/bottom = 4 }/bottom = 4, stretch = "tile_fit" }/' \
    "$project_dir/window.px.toml" > "$sheet_tile_fit_window"

"$pxsmith" nineslice preview \
    "$project_dir/window.px.toml" \
    "$project_dir/window.png" \
    --size 12x12
"$pxsmith" nineslice preview \
    "$project_dir/window.px.toml" \
    "$project_dir/window-stretch-30x22.png" \
    --size 30x22
"$pxsmith" nineslice preview \
    "$tile_window" \
    "$project_dir/window-tile-30x22.png" \
    --size 30x22
"$pxsmith" nineslice preview \
    "$tile_fit_window" \
    "$project_dir/window-tile-fit-30x22.png" \
    --size 30x22

"$pxsmith" sheet pack \
    "$project_dir/ui.png" \
    --input \
    "$sheet_stretch_window" \
    "$sheet_tile_window" \
    "$sheet_tile_fit_window" \
    "$repo_dir/examples/sprites/hero_body.px.toml" \
    --meta "$project_dir/ui.sheet.json"
"$pxsmith" export godot \
    --sheet "$project_dir/ui.sheet.json" \
    --out-dir "$project_dir" \
    --res-dir res://

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
