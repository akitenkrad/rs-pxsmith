# Command line

**English** | [日本語](cli.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

The examples below invoke an installed `pxsmith` binary. When running from a
checkout, prefix them with `cargo run -p pxsmith --`.

`.px.toml` is the L0 text format, in which a sprite is written as characters and
the palette is held in a separate `.hex` file. `.aseprite` files round-trip byte
for byte, so pxsmith can sit in the middle of an existing Aseprite workflow
without taking ownership of the file.

L0 anchors in `[meta]` are defaults shared by every frame. A `[[frame]]` may
override any of them without repeating the rest:

```toml
[meta]
format = 1
name = "body"
anchors = { head = [8, 3], feet = [8, 15] }

[[frame]]
name = "step"
anchors = { head = [8, 2] }
data = '''
0
'''
```

Here `step` uses `[8, 2]` for `head` and retains the `[8, 15]` default for
`feet`. Frames without an `anchors` field keep the existing byte-exact canonical
form, and the L0 format version remains 1.

## Basics

```sh
# Check the terminal can show pixels accurately (Kitty / iTerm2 / Sixel)
pxsmith verify terminal

# Convert a sprite layer to editable text, and back
pxsmith text export sprite.aseprite hero.px.toml --palette pal.hex
pxsmith text import hero.px.toml sprite.aseprite

# Watch a file and redraw on save
pxsmith watch hero.px.toml --zoom 8

# Show which pixels changed between two sprites
pxsmith diff before.px.toml after.px.toml

# Inspect or convert palettes (.hex is the canonical format)
pxsmith palette info palettes/sweetie-16.hex
pxsmith palette convert input.gpl output.hex

# Check that reading and writing an .aseprite file is byte-exact
pxsmith verify roundtrip sprite.aseprite --via-frame
```

## Deriving artwork

Shading is derived from the silhouette rather than painted, and the source
colours of the input are discarded. This is what keeps flips, tweens, and
recolours from breaking the light direction.

```sh
# Derive shading from a silhouette (the source colors are discarded)
pxsmith shade hero.png hero.px.toml --base 8A6A4A --light dir:-0.6,0.8

# Normalize jaggies, add antialiasing, draw an outline
pxsmith smooth hero.px.toml smoothed.px.toml
pxsmith aa smoothed.px.toml aa.px.toml
pxsmith outline aa.px.toml outlined.px.toml --style tinted

# Animation: inbetweens, timing, cycles, smears, squash, subpixel, afterimages
pxsmith anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
pxsmith anim smear out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim squash in.px.toml out.px.toml --amount -0.3
pxsmith anim subpixel in.px.toml out.px.toml --method tangent

# Check the result against hardware constraints (non-zero exit on violation)
pxsmith lint out.px.toml
pxsmith validate out.px.toml --target gb
```

### Colour reduction

```sh
pxsmith quantize photo.png indexed.png --colors 16 --method kmeans
pxsmith clean indexed.png cleaned.png
pxsmith conform upscaled.png native.png
```

`conform` recovers the original grid of an image that was scaled up, and possibly
JPEG-compressed, and returns it to 1:1. When the grid is not uniform it refuses
rather than guessing, because a non-uniform grid cannot be undone
deterministically and the image belongs back with a person.

## Composition, tilesets, and projection

```sh
# Extract resolved per-frame anchor positions, then apply the track to another L0 file
pxsmith anchor track body.px.toml --anchor head -o head-track.json
pxsmith anchor apply head-track.json target.px.toml -o tracked.px.toml --anchor head

# Assemble parts, then derive the other seven directions by flip + re-shading
pxsmith compose out.px.toml --part body.px.toml --part head.px.toml


pxsmith direction 'out/${dir}.px.toml' --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade

# Cut a sheet into tiles, collapse duplicates, build a 47-piece autotile set
# (indexed input only — quantising here would smuggle our choice into tile equality)
pxsmith tileset extract sheet.aseprite tiles.aseprite --tile 16 --map map.json
pxsmith tileset autotile quadrants.px.toml auto.aseprite

# Fake depth: haze distant layers toward the sky, record parallax speeds
pxsmith atmos 'out/${name}.px.toml' --input fg.px.toml --input bg.px.toml \
    --sky 41a6f6 --haze background=0.6 --scroll-meta out/scene.scroll.json

# Project a top-down sprite onto an isometric floor, and draw a matching guide
pxsmith project in.px.toml iso.px.toml --to iso --from top --facing right
pxsmith guide g.png --projection iso --from top --cell 16 --size 256x256
```

`anchor track` writes resolved values (frame override first, then the `[meta]`
default) as format-1 JSON. With no `--anchor`, it writes every name in sorted
order; with no `-o`, it writes JSON to standard output. `anchor apply` requires
the track and target to have exactly the same frame count, writes values into
each target frame, and leaves `[meta]` unchanged. Anchor commands accept only L0
`.px.toml` artwork.

Composition resolves offsets separately for every output frame. Each part uses
the anchor on the frame currently selected by its delay mode, while a delayed
part still follows the base part's current visible frame. The canvas and origin
are the union across all parts and all output frames, so a moving anchor cannot
make packed frame cells change size. If offsets vary, the report shows the
frame-zero offset and the x/y ranges; constant offsets retain the previous
single-offset report.

`project` requires both `--from` and `--facing`. Which face is being laid down,
and which way it points, cannot be read off the pixels, so a guess would fail
silently whenever it was wrong.

## Scaling and rotation

```sh
pxsmith scale in.px.toml out.px.toml --factor 4          # nearest by default (exact)
pxsmith rotate in.px.toml out.px.toml --degrees 30 --algo cleanedge
```

Integer scales and quarter turns are implemented as index substitution rather
than sampling, so four quarter turns return the original image exactly.
`cleanedge` pays off when a rotation is combined with an upscale; at 1:1 the
default `nearest` gives the better result, and the CLI says so at run time.

## Exporting

```sh
pxsmith sheet pack out/sheet.png --input a.px.toml --input b.px.toml --meta out/sheet.sheet.json
pxsmith export tiled map.json --tsx out/sheet.tsx --sheet out/sheet.sheet.json
```

`sheet pack` normally merges the colours used by every input into one palette.
Use palette-swap output when that merged palette would exceed 256 colours, or
when the game should select colours in a shader:

```sh
pxsmith sheet pack out/sheet.png \
  --input hero.px.toml --input enemy.px.toml \
  --palette-swap \
  --variant palettes/hero.hex=palettes/hero-red.hex \
  --variant palettes/hero.hex=palettes/hero-blue.hex
```

This writes two RGBA8 PNG files and format 2 sheet metadata:

- `sheet.png` is an index texture. Its red channel is the unchanged source
  index, green and blue are zero, and alpha is 255. Source-transparent pixels,
  palette entries with alpha zero, padding, margins, and unused cells are
  `(0, 0, 0, 0)`.
- `sheet.palettes.png` is a palette atlas. Each distinct input palette becomes
  one row in first-input order; byte-identical palettes share a row. Variant
  rows follow in command-line order. The atlas width is the longest palette,
  and the unused tail of shorter rows is transparent.
- `sheet.sheet.json` has `format: 2`. Each cell names its base palette row, and
  the `palettes.rows` array records row names and each variant's base row.
  Format 1 remains the normal merged-palette contract.

`--variant BASE.hex=VARIANT.hex` requires an input row exactly equal to BASE.
VARIANT must have the same length and the same transparent positions. Row names
come from variant file stems and must be unique. Palette-swap output is PNG-only;
it cannot be written as `.aseprite`.

To make a normal RGBA image with the same lookup performed by a shader, use:

```sh
pxsmith sheet resolve out/sheet.png --meta out/sheet.sheet.json -o out/resolved.png
pxsmith sheet resolve out/sheet.png --meta out/sheet.sheet.json \
  --variant hero-red -o out/resolved-red.png
```

Without `--variant`, every cell uses its own base row. With a variant name, only
cells whose base row is that variant's `variant_of` row are changed. `export
tiled` still uses format 2 cell geometry, but warns that Tiled displays the
index texture rather than resolved colours.

See [`examples/godot-palette-swap/`](../examples/godot-palette-swap/) for a
Godot 4.7 canvas shader, exact texture import settings, three row selections,
and a command-line screenshot mode.

## Inspecting

```sh
pxsmith view walk.px.toml --frame 2 --onion 2   # onion skin, outlines only
pxsmith palette report hero.px.toml --top 12    # which colours carry the area
pxsmith palette cvd palettes/sweetie-16.hex     # Protan and Deutan checks
```

`palette report` gives four coverage thresholds rather than one, and reports the
total area of an index separately from its largest connected blob. A colour
scattered across a sprite is not a main colour, but reading the total alone would
call it one.

### Colour vision deficiency checks

`palette cvd` accepts a `.hex` palette or `.px.toml`, `.aseprite`, and `.png`
artwork. For artwork, only colours that are actually used are checked;
completely transparent colours are ignored. Protan and Deutan are checked by
default. Pass a comma-separated list to select other types:

```sh
pxsmith palette cvd hero.px.toml --type protan,deutan,tritan
pxsmith palette cvd hero.px.toml --pair ffcd75,a7f070 --json
```

The simulation uses the severity 1.0 matrices from Machado, Oliveira and
Fernandes, “A Physiologically-based Model for Simulation of Color Vision
Deficiency” (IEEE TVCG 15(6), 2009). The matrices are applied to linear RGB:
sRGB is decoded before the matrix and encoded again afterwards. Pair distances
use the same Euclidean OKLab distance as the other pxsmith colour tools.

The default `--threshold 0.02` is one just-noticeable difference in OKLCh as
defined by W3C CSS Color Module Level 4 §14.2.1. Pairs already below the
threshold without simulation are counted separately, because their ambiguity
was not caused by colour vision deficiency.

Without `--pair`, findings are advisory and the command exits with status 0.
Repeat `--pair RRGGBB,RRGGBB` for colour pairs that carry game meaning; if one
of those pairs becomes indistinguishable, the command reports it as blocking
and exits with status 1. A named colour that is not in the checked palette is an
input error with status 2. Clap syntax and value errors also use status 2;
other I/O or processing errors use status 1.

## Building from a checkout

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

`cargo-make` tasks are defined in `Makefile.toml`.

```sh
cargo make format-all   # taplo + clippy + rustfmt
cargo make test
```
