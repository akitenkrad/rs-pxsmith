# Command line

**English** | [日本語](cli.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

The examples below invoke an installed `pxsmith` binary. When running from a
checkout, prefix them with `cargo run -p pxsmith --`.

`.px.toml` is the L0 text format, in which a sprite is written as characters and
the palette is held in a separate `.hex` file. `.aseprite` files round-trip byte
for byte, so pxsmith can sit in the middle of an existing Aseprite workflow
without taking ownership of the file.

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
pxsmith sheet pack out/sheet.png --input a.px.toml --input b.px.toml --layout out/sheet.json
pxsmith export tiled map.json map.tmx --sheet out/sheet.json
```

## Inspecting

```sh
pxsmith view walk.px.toml --frame 2 --onion 2   # onion skin, outlines only
pxsmith palette report hero.px.toml --top 12    # which colours carry the area
```

`palette report` gives four coverage thresholds rather than one, and reports the
total area of an index separately from its largest connected blob. A colour
scattered across a sprite is not a main colour, but reading the total alone would
call it one.

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
