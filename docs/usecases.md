# Use cases

**English** | [日本語](usecases.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

pxsmith is aimed at the work that surrounds drawing rather than the drawing
itself. Each case below states the problem, the approach the tool takes, and the
commands that carry it out. Full flag documentation is in
[the command line reference](cli.md).

---

## 1. Reviewing sprite changes in a pull request

### The problem

A binary `.aseprite` file cannot be diffed, so a change to a sprite arrives in
review as nothing more than "the file changed". A reviewer cannot see which
pixels moved without opening the editor.

### The approach

Convert the layer to L0 text and review that instead. The round trip is
byte-exact, so the text can be the reviewed artefact while the `.aseprite` stays
the working file.

<p align="center"><img src="assets/usecases/review.svg" width="100%" alt=""></p>

`diff` counts changed pixels and reports their positions individually rather than
as a summary statistic, because in pixel art a single pixel carries meaning.

### Commands

```sh
pxsmith text export hero.aseprite hero.px.toml --palette pal.hex
pxsmith diff old.px.toml hero.px.toml
pxsmith text import hero.px.toml hero.aseprite
```

---

## 2. Deriving eight directions from one drawing

### The problem

Drawing a character in eight directions means keeping eight sprites consistent
with each other. Mirroring one of them is not enough on its own, because the
shading mirrors with it and the light appears to move.

### The approach

Draw one direction and derive the rest. Shading is derived rather than painted,
so a mirrored sprite can be re-shaded from the original light direction.

<p align="center"><img src="assets/usecases/direction.svg" width="100%" alt=""></p>

### Commands

```sh
pxsmith direction 'out/${dir}.px.toml' --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade
```

The output path must contain `${dir}`, since one file is written per direction.

---

## 3. Recovering artwork that arrived upscaled

### The problem

Pixel art collected from the web, exported at the wrong zoom, or produced by an
image model usually arrives scaled up and often JPEG-compressed. Scaling it back
down by eye loses the original grid, and the colour count has already exploded.

### The approach

Recover the grid, return the image to 1:1, then reduce it to a workable indexed
palette.

<p align="center"><img src="assets/usecases/conform.svg" width="100%" alt=""></p>

When the grid is not uniform, `conform` refuses rather than guessing, because a
non-uniform grid cannot be undone deterministically.

### Commands

```sh
pxsmith conform upscaled.png native.png
pxsmith quantize native.png indexed.png --colors 16 --method kmeans
pxsmith clean indexed.png cleaned.png
```

---

## 4. Building a tileset from a hand-drawn sheet

### The problem

Cutting a sheet into tiles and removing duplicates is mechanical work that is
tedious by hand and easy to get subtly wrong. Building a 47-piece autotile set
from quadrants is worse.

### The approach

Collapse equivalent tiles automatically and write a map alongside, then build the
autotile set from quadrants and export to the map editor.

<p align="center"><img src="assets/usecases/tileset.svg" width="100%" alt=""></p>

Input must already be indexed. Quantising at this point would let the tool's own
colour choices decide which tiles count as equal.

### Commands

```sh
pxsmith tileset extract sheet.aseprite tiles.aseprite --tile 16 --map map.json
pxsmith tileset autotile quadrants.px.toml auto.aseprite
pxsmith export tiled map.json map.tmx --sheet out/sheet.json
```

---

## 5. Filling in animation between two keyframes

### The problem

Inbetweens are repetitive to draw, and the errors they introduce are hard to see
one frame at a time. A line that wobbles, or a dither pattern that stays fixed to
the canvas while the object moves, only shows up in motion.

### The approach

Compute the inbetweens from the two keys, adjust timing separately, and let the
checker look across the sequence rather than at single frames.

<p align="center"><img src="assets/usecases/anim.svg" width="100%" alt=""></p>

### Commands

```sh
pxsmith anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
pxsmith anim squash in.px.toml out.px.toml --amount -0.3
pxsmith lint out.px.toml
```

---

## 6. Checking artwork against hardware constraints

### The problem

Retro-platform projects have palette and tile limits that are easy to exceed
while drawing and awkward to discover once the art is finished.

### The approach

Check the constraint as part of the build. The command exits non-zero on a
violation, so it drops into CI as it stands.

<p align="center"><img src="assets/usecases/validate.svg" width="100%" alt=""></p>

Built-in targets are `gb`, `nes`, `snes`, `gba`, and `pico8`, and a TOML profile
can be supplied for anything else.

### Commands

```sh
pxsmith validate hero.px.toml --target gb
pxsmith validate hero.px.toml --target nes --json
```

---

## 7. Running an asset pipeline in CI

### The problem

A derivation kept as a shell script drifts between machines, and re-running it
rebuilds everything whether or not anything changed.

### The approach

Describe the derivation as data. Step keys resolve incrementally, so an unchanged
step is known to be unchanged without running it.

<p align="center"><img src="assets/usecases/recipe.svg" width="100%" alt=""></p>

Across 128 steps over 64 sprites this takes 2.66 seconds cold and 0.09 seconds
warm, and changing one input rebuilds exactly the two steps that depend on it.
Builds are byte-identical across thread counts, which is verified by a test
rather than asserted. See [Recipes](recipes.md).

### Commands

```sh
pxsmith run build.toml --dry-run   # show the order, run nothing
pxsmith run build.toml
```

---

## 8. Producing placeholder artwork while a game is prototyped

### The problem

Prototyping stalls while waiting for art, but generated art usually arrives in
the wrong palette, at the wrong scale, or with colours the project never
declared.

### The approach

Have the model write palette indices rather than colours, then verify the result
with the same lint used on hand-drawn work and ask again when it fails.

<p align="center"><img src="assets/usecases/gen.svg" width="100%" alt=""></p>

Because the palette lives in a separate `.hex` file that the tool writes first,
a generated sprite cannot introduce a colour the project has not declared. See
[Generation](generation.md) for what the loop does and does not verify.

### Commands

```sh
export ANTHROPIC_API_KEY=...
pxsmith gen prog out/chest.px.toml --prompt "a wooden chest, seen head-on" \
    --palette 1a1c2c,566c86,8a6a4a,b13e53,f4f4f4 --size 16x16
```

---

## 9. Drawing with immediate feedback in a terminal

### The problem

Editing L0 text in an editor means losing sight of the sprite, and not every
terminal can show pixels faithfully enough to judge them.

### The approach

Redraw on every save, and check first whether the terminal is good enough to
trust.

<p align="center"><img src="assets/usecases/watch.svg" width="100%" alt=""></p>

`verify terminal` answers whether the terminal is good enough to judge single
pixels rather than merely whether it can show an image at all. The half-block
fallback halves the vertical resolution and does not qualify.

### Commands

```sh
pxsmith verify terminal
pxsmith watch hero.px.toml --zoom 8
pxsmith view walk.px.toml --frame 2 --onion 2
```

---

## 10. Using the crates without the CLI

### The problem

A project with its own build system does not want to shell out to a binary, and
sprites referenced by path can go missing without the compiler noticing.

### The approach

Call the same operations as library functions, and embed sprites at compile time
so that a malformed one becomes a compile error.

<p align="center"><img src="assets/usecases/library.svg" width="100%" alt=""></p>

### Commands

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

Row-length mismatches become compile errors, and editing the referenced palette
triggers a rebuild. See [Library](library.md) for the crate split, including how
to take `pxsmith-view` without its terminal backend.

---

## What pxsmith is not for

It has no drawing interface and no canvas, and it will not be a replacement for
Aseprite or any other editor. It also does not decide questions that belong to
the artist: `conform` refuses a non-uniform grid instead of guessing, `project`
requires the projection to be stated rather than inferred, and `palette report`
gives four coverage thresholds rather than picking one.
