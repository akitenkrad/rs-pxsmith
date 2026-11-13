# Godot palette swap example

This Godot 4.7 project displays the same 16×16 sheet cell three times: the base
palette row, the `red` variant, and the `blue` variant. Each `Sprite2D` owns a
separate `ShaderMaterial`, uses `region_enabled` to select the first cell, and
is positioned and scaled by integers.

The canvas shader reads the red channel of `sheet.png` as an exact 8-bit index.
Transparent pixels are discarded, and `texelFetch` reads that index from the
selected row of `sheet.palettes.png`. The atlas rows are recorded in
`sheet.sheet.json`: row 0 is `sample`, row 1 is `red`, and row 2 is `blue`.

## Rebuilding the assets

From this directory, run:

```sh
./make_assets.sh
```

Set `PXSMITH=/path/to/pxsmith` to use an existing binary. Otherwise the script
builds the workspace binary when needed. It packs
`../sprites/hero_body.px.toml` with `--palette-swap` and both variants.

## Running and capturing

Open `project.godot` in Godot 4.7, or run:

```sh
godot --path examples/godot-palette-swap
godot --path examples/godot-palette-swap -- --capture /tmp/palette-swap.png
```

The capture mode waits for three rendered frames, saves the viewport, and
quits. Check that all three sprites have the same silhouette and visibly
different base, red, and blue colours.

## Import settings

Both PNG files include explicit Godot 4 `.import` files. On first import,
confirm that Godot keeps these settings:

- Lossless compression (`compress/mode=0`) preserves index channel values.
- Mipmaps are disabled so downsampled indices are never produced.
- The project default and shader sampling are nearest-neighbour; interpolation
  between adjacent indices would select a different palette entry.
- `process/fix_alpha_border=false` prevents Godot from rewriting transparent
  border RGB values.
- `process/premult_alpha=false` prevents RGB values from being multiplied by
  alpha.

These constraints matter most for `sheet.png`: compression, filtering, colour
processing, or alpha processing can change its red channel, which is data rather
than display colour. The palette sampler deliberately has no `source_color`
hint, avoiding an sRGB-to-linear conversion before the lookup result is used.

## Verification

The example was run on Godot 4.7.2 (macOS, `gl_compatibility`). A first
`godot --headless --path . --import` kept every import setting above; Godot only
added a `uid` line to each `.import` file. The capture was then compared with
`pxsmith sheet resolve` for the base, `red`, and `blue` rows: all 256 source
pixels of each sprite matched exactly, and every 8×8 block of the scaled capture
was a single colour. To repeat the check:

```sh
godot --path examples/godot-palette-swap -- --capture /tmp/palette-swap.png
pxsmith sheet resolve examples/godot-palette-swap/sheet.png \
  --meta examples/godot-palette-swap/sheet.sheet.json --variant red -o /tmp/red.png
```
