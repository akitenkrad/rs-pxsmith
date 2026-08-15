# Use cases

**English** | [日本語](usecases.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

pxsmith is aimed at the work that surrounds drawing rather than the drawing
itself. The cases below are the ones the tool was built for, each with the
commands that carry it out. Full flag documentation is in
[the command line reference](cli.md).

## 1. Reviewing sprite changes in a pull request

A binary `.aseprite` file cannot be diffed, so a change to a sprite arrives in
review as "the file changed". Converting the layer to L0 text makes the change
readable, and the round trip is byte-exact, so the text can be the reviewed
artefact while the `.aseprite` stays the working file.

```sh
pxsmith text export hero.aseprite hero.px.toml --palette pal.hex
pxsmith diff old.px.toml hero.px.toml
```

`diff` counts changed pixels and reports their positions individually rather
than as a summary statistic, because in pixel art a single pixel carries
meaning.

## 2. Deriving eight directions from one drawing

Drawing a character once and deriving the remaining directions saves the work of
keeping eight sprites consistent. Because shading is derived rather than painted,
a mirrored sprite can be re-shaded so the light keeps coming from the same place
instead of flipping with the artwork.

```sh
pxsmith direction 'out/${dir}.px.toml' --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade
```

The output path must contain `${dir}`, since one file is written per direction.

## 3. Recovering artwork that arrived upscaled

Pixel art collected from the web, exported from a tool at the wrong zoom, or
produced by an image model usually arrives scaled up and often JPEG-compressed.
`conform` recovers the original grid and returns the image to 1:1, then
`quantize` and `clean` bring it back to a workable indexed palette.

```sh
pxsmith conform upscaled.png native.png
pxsmith quantize native.png indexed.png --colors 16 --method kmeans
pxsmith clean indexed.png cleaned.png
```

When the grid is not uniform, `conform` refuses rather than guessing. A
non-uniform grid cannot be undone deterministically, so the image goes back to a
person instead of being silently mangled.

## 4. Building a tileset from a hand-drawn sheet

Cutting a sheet into tiles and removing duplicates is mechanical work that is
tedious to do by hand and easy to get subtly wrong. `tileset extract` collapses
equivalent tiles and writes a map, and `tileset autotile` builds the 47-piece set
from quadrants.

```sh
pxsmith tileset extract sheet.aseprite tiles.aseprite --tile 16 --map map.json
pxsmith tileset autotile quadrants.px.toml auto.aseprite
pxsmith export tiled map.json map.tmx --sheet out/sheet.json
```

Input must already be indexed. Quantising at this point would let the tool's own
colour choices decide which tiles count as equal.

## 5. Filling in animation between two keyframes

Inbetweens, timing, and the secondary motion around them are computed from the
two keys rather than drawn.

```sh
pxsmith anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
pxsmith anim squash in.px.toml out.px.toml --amount -0.3
```

Six of the 27 rules apply across a frame sequence, so `lint` will report a
sequence whose topology changes between frames, whose line wobbles, or whose
dither sticks to the canvas instead of travelling with the object.

## 6. Checking artwork against hardware constraints

Retro-platform projects have palette and tile limits that are easy to exceed and
awkward to discover late.

```sh
pxsmith validate hero.px.toml --target gb
pxsmith validate hero.px.toml --target nes --json
```

Built-in targets are `gb`, `nes`, `snes`, `gba`, and `pico8`, and a TOML profile
can be supplied for anything else. The command exits non-zero on a violation, so
it drops into CI as it stands.

## 7. Running an asset pipeline in CI

A recipe describes the whole derivation as data, so the same build runs on a
developer machine and on a build server, and re-running it rebuilds only what
changed.

```sh
pxsmith run build.toml --dry-run   # show the order, run nothing
pxsmith run build.toml
```

Across 128 steps over 64 sprites this takes 2.66 seconds cold and 0.09 seconds
warm. Builds are byte-identical across thread counts, which is verified by a test
rather than asserted. See [Recipes](recipes.md).

## 8. Producing placeholder artwork while a game is prototyped

Placeholder sprites can be generated from a prompt and a palette, then verified
by the same lint used on hand-drawn work. The model writes palette indices
rather than colours, so a generated sprite cannot introduce a colour that the
project has not declared.

```sh
export ANTHROPIC_API_KEY=...
pxsmith gen prog out/chest.px.toml --prompt "a wooden chest, seen head-on" \
    --palette 1a1c2c,566c86,8a6a4a,b13e53,f4f4f4 --size 16x16
```

See [Generation](generation.md) for what the repair loop does and does not
verify.

## 9. Drawing with immediate feedback in a terminal

`watch` redraws the sprite on every save, which suits editing L0 text directly in
an editor.

```sh
pxsmith verify terminal        # is this terminal pixel-accurate?
pxsmith watch hero.px.toml --zoom 8
pxsmith view walk.px.toml --frame 2 --onion 2
```

`verify terminal` answers whether the terminal is good enough to judge single
pixels rather than merely whether it can show an image at all. Kitty, iTerm2, and
Sixel qualify; the half-block fallback halves the vertical resolution and does
not.

## 10. Using the crates without the CLI

Every operation above is a library function, so a project with its own build
system can call them directly. Sprites can also be embedded at compile time,
where a row-length mismatch becomes a compile error.

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

See [Library](library.md) for the crate split, including how to take
`pxsmith-view` without its terminal backend.

## What pxsmith is not for

It has no drawing interface and no canvas, and it will not be a replacement for
Aseprite or any other editor. It also does not decide questions that belong to
the artist: `conform` refuses a non-uniform grid instead of guessing, `project`
requires the projection to be stated rather than inferred, and `palette report`
gives four coverage thresholds rather than picking one.
