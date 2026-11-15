# Command line

**English** | [日本語](cli.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

The examples below invoke an installed `pxsmith` binary. When running from a
checkout, prefix them with `cargo run -p pxsmith --`.

`.px.toml` is the L0 text format, in which a sprite is written as characters and
the palette is held in a separate `.hex` file. `.aseprite` files round-trip byte
for byte, so pxsmith can sit in the middle of an existing Aseprite workflow
without taking ownership of the file.

Commands that write a sequence of frames (the `anim` commands, `compose`,
`direction`, `scale`, `rotate` and the like) choose the format from the output
name: `.px.toml` writes L0 with its palette beside it, `.gif` writes an animated
GIF, and `.aseprite` writes an Aseprite file. A GIF is written at actual size and
loops forever. Transparent pixels become the GIF transparent colour, and each
frame's colours are mapped exactly into its own local colour table, so nothing
is requantised; a frame that would need more than 256 colours, transparency
included, is an error. GIF delays have 10 ms steps, so durations are rounded and
the command reports how many frames were rounded and to what. GIF is an output
format only: no command reads it back, so apply timing with `anim ease` before
the GIF is written.

L0, PNG and GIF can hold only one visible layer per frame. When a command such
as `compose` produces several layers, these formats flatten them from bottom to
top and print one line saying so; `.aseprite` keeps the layers. L0 flattening
copies existing indexed colours exactly and never creates a blended colour. A
partly transparent layer or a non-normal blend mode is therefore an explicit
error instead of silently adding a colour that was not in the palette.

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

# Inspect or convert palettes
pxsmith palette info palettes/sweetie-16.hex
pxsmith palette convert input.gpl output.hex
pxsmith palette convert output.hex output.gpl

# Check that reading and writing an .aseprite file is byte-exact
pxsmith verify roundtrip sprite.aseprite --via-frame
```

Palette input may be `.hex`, `.gpl`, `.pal` or `.act`. Conversion output is
selected by its extension: `.hex` writes the canonical line-per-colour form and
`.gpl` writes a GIMP Palette. Other output extensions are rejected.

## Deriving artwork

Shading is derived from the silhouette rather than painted, and the source
colours of the input are discarded. This is what keeps flips, tweens, and
recolours from breaking the light direction.

For every `--light` option that accepts a geometric source, `dir:x,y` is the
direction in which light travels; positive x points right and positive y points
down. `dir:-0.6,0.8` therefore comes from the upper right, while
`dir:0.6,0.8` comes from the upper left. The default is `dir:-0.6,0.8`. In
`point:x,y`, x and y are the light source's position instead. `atmos --light`
is the exception: it accepts a colour preset, not a direction.

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

# Rolling digits, as a score counter does (see "Rolling digits" below)
pxsmith anim roll out/misaki.pxfont.toml --from 0129 --to 0131 --frames 4 \
    out/score-roll.px.toml --palette palettes/sweetie-16.hex --ink 12

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

# Assemble parts, then draw n/ne/e/se/s and reflect the remaining three
pxsmith compose out.px.toml --part body.px.toml --part head.px.toml
pxsmith direction 'out/${dir}.px.toml' \
    --from n=hero_n.px.toml --from ne=hero_ne.px.toml --from e=hero_e.px.toml \
    --from se=hero_se.px.toml --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade --base 8A6A4A

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

`direction` can reflect only mirror partners. With n, ne, e, se and s drawn, it
creates sw, w and nw. It reports and skips any direction that cannot be made by
reflection; it does not invent the other seven from one drawing.

`--light` declares the light for lint rule 7, which is applied to the reflected
directions. With `--reshade` it is also the light used to re-derive the shading,
computed exactly as `pxsmith shade --light` does; without `--light`, re-shading
uses the default light of `--preset`.

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

For `.px.toml`, `.png` and `.gif` output, `compose` flattens the part layers in
their declared order, with later parts covering earlier ones and transparent
pixels leaving lower parts visible. The result is pixel-identical to the
layered `.aseprite` output at actual size. `.aseprite` output continues to keep
one layer per part.

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

## Fonts and text

Pixel fonts are kept in their own text format, `.pxfont.toml`, rather than in
L0. An existing BDF, TTF, OTF or TTC font is imported once; after that the font
is checked against the text a game actually uses, text is rendered with it into
artwork, and it is exported as an AngelCode BMFont for a game engine.

```sh
# Import a BDF font, or only the characters a text file uses
pxsmith font import testdata/fonts/misaki/misaki_gothic.bdf out/misaki.pxfont.toml
pxsmith font import testdata/fonts/misaki/misaki_gothic.bdf out/ui.pxfont.toml \
    --chars-file script.txt

# Import TTF/OTF/TTC fonts at a given pixel size
pxsmith font import testdata/fonts/misaki/misaki_gothic.ttf out/misaki-ttf.pxfont.toml --size 8
pxsmith font import testdata/fonts/synthetic/pxsynth-gpos.ttf out/gpos.pxfont.toml --size 8

# Check that the font has every character the text needs (exit status 1 if not)
pxsmith font check out/ui.pxfont.toml --text-file script.txt
pxsmith font check out/misaki.pxfont.toml --text-file script.txt menu.txt
pxsmith font check out/misaki.pxfont.toml --text-file menu.txt --vertical

# Render text, then export the font for an engine
pxsmith font render out/misaki.pxfont.toml "SCORE 0120" out/score.px.toml \
    --palette palettes/sweetie-16.hex --ink 12
pxsmith font bmfont out/misaki.pxfont.toml out/misaki.fnt
```

### The `.pxfont.toml` format

A font is one file. Each glyph is drawn as a whole cell, its advance wide and
the font's line height tall, so the baseline position is fixed by the cell
rather than worked out from offsets by hand. Trimming to the ink box is left to
the exporters.

```toml
[meta]
format = 1
name = "tiny"
line_height = 3
baseline = 2
default_advance = 3

[[glyph]]
char = "U+0020"
data = '''
..
..
..
'''

[[glyph]]
char = "A"
data = '''
.#.
###
#.#
'''

[[glyph]]
char = "ー"
variant = "vert"
data = '''
.#.
.#.
.#.
'''

[[kern]]
pair = "AV"
amount = -1
```

- `[meta]` holds `format` (always 1), `name`, `line_height` (the cell height in
  pixels), `baseline` (pixels from the top of the cell to the baseline, at most
  `line_height`) and `default_advance`. `source` and `copyright` are optional
  and record where the font came from; importers fill them in.
- Glyphs are single-colour by default: `#` is ink and `.` is transparent, and
  the ink colour is chosen when the text is rendered. A colour font declares
  `mode = "color"` in `[meta]` and carries the same `[palette]` table as L0
  (`ref` to a `.hex` file, and a `map` of one-character keys, with `.` for
  transparent). The two modes are not mixed in one font.
- `char` is one character or `U+XXXX`. The canonical form writes whitespace
  and control characters as `U+XXXX` and everything else as the character
  itself.
- `data` must have exactly `line_height` rows of equal width. `advance` is
  optional; a glyph without one advances by the width of its `data`, so a
  narrow glyph drawn narrow needs nothing more. `default_advance` is used only
  for a glyph whose `data` is empty. The canonical form writes `advance` only
  when it differs from that.
- `variant = "vert"` marks the form a character takes in vertical text. The
  same character and variant may appear only once.
- `[[kern]]` adjusts the horizontal advance between a pair of exactly two
  characters by `amount` pixels. Each pair may appear only once.
- The canonical form orders glyphs by code point with the base glyph before its
  variants, and kerning pairs by code point. `mode = "mono"` is accepted but not
  written.

### Importing fonts

`font import` chooses the reader from the input extension (`.bdf`, `.ttf`,
`.otf` or `.ttc`) and reports how many characters were imported, skipped and
clipped. The font's copyright notice is copied into `copyright` and printed.

- **BDF 2.1.** `line_height` and `baseline` come from `FONT_ASCENT` and
  `FONT_DESCENT`, and each glyph is placed in its cell from its `BBX`. Fonts
  registered as `ISO10646` are read directly; `JISX0208` fonts are mapped to
  Unicode through the Unicode Consortium's `JIS0208.TXT` table, which ships
  with `pxsmith-io`. Glyphs without a code point (`ENCODING -1`) and characters
  the table cannot map are skipped and counted.
- **TTF, OTF and TTC.** `--size` (pixels per em) is required, and `--face N`
  selects a face in a collection (default 0). A 1-bit embedded bitmap strike
  at exactly that size is used as it is. Otherwise the outline is rasterised
  without antialiasing: a pixel is ink when its centre lies inside the outline
  by the nonzero winding rule, so a pixel font drawn on the grid comes back dot
  for dot. `--outline` forces the outline path even when a strike exists. The
  share of glyphs whose outline points are off the pixel grid is reported, with
  advice when it exceeds 5%, because a font not drawn for pixels will be
  damaged by this rasterisation.
- GSUB `vert`/`vrt2` single substitutions become `variant = "vert"` glyphs
  (`vrt2` wins when both apply). GPOS pair kerning becomes `[[kern]]`, rounded
  to pixels; the `kern` table is used only when the font has no GPOS kerning.
- A glyph whose ink falls outside its cell makes the whole import fail and is
  listed. `--clip` cuts it to the cell instead and counts the clipped glyphs.
- `--chars-file FILE` imports only the characters that appear in that file
  (line breaks are ignored).

Misaki Gothic's TTF carries both an 8 ppem bitmap strike and outlines; both
paths reproduce the BDF import for all 7,171 glyphs.

### Checking a font against the text

`font check` reads one or more text files and fails with exit status 1 when
the font lacks any character they use, listing the characters. With
`--vertical` it also fails on characters that need a vertical form
(`Vertical_Orientation` Tu or Tr) but have no `variant = "vert"` glyph.

Shape advice is printed but does not change the exit status: glyphs with ink in
their right-most column, which touch the next glyph when that glyph has ink in
its left-most column (box drawing, block elements and vertical forms are
excluded, since they are meant to join), glyphs with no ink at all (whitespace
excluded), and glyphs whose advance is narrower than their `data`.

### Rendering text

`font render` lays text out and writes it on a transparent background as
`.px.toml`, `.aseprite` or `.png`. The text is either a positional argument
before the output, or a UTF-8 file given with `--text-file`, in which case only
the output is positional.

```sh
# Wrapped dialogue, centred, with hanging punctuation
pxsmith font render out/misaki.pxfont.toml --text-file script.txt out/script.png \
    --palette palettes/sweetie-16.hex --ink 12 --wrap 64 --hang --line-spacing 2 --align center

# Vertical text in 64 px columns, with two-digit numbers set side by side
pxsmith font render out/misaki.pxfont.toml --text-file script.txt out/script-v.png \
    --palette palettes/sweetie-16.hex --ink 12 --vertical --wrap 64 --tcy 2

# Kerning: the GPOS pair AV is one pixel tighter (use --no-kern to compare)
pxsmith font render out/gpos.pxfont.toml AV out/av.png --palette palettes/sweetie-16.hex --ink 12
```

- A single-colour font needs `--palette` and `--ink`, the palette index used for
  the ink. A colour font keeps its glyph colours and rejects `--ink`. Without
  `--palette` it uses the font's own palette; with one, every colour of the
  glyphs actually drawn must be in it, and missing colours are listed. A
  transparent entry is added to the palette when it has none.
- `--letter-spacing` and `--line-spacing` add pixels between characters and
  between lines. `--align left|center|right` aligns lines within the widest
  line.
- Kerning pairs from `[[kern]]` are applied to horizontal text unless
  `--no-kern` is given.
- A character the font lacks is an error that lists every such character.
  `--missing C` (one character, or `U+XXXX`) substitutes C instead and reports
  how many were replaced.
- Line breaks in the text are kept. `--wrap N` wraps lines at N pixels, with
  break opportunities taken from UAX #14 (the Unicode line breaking algorithm)
  through the `unicode-linebreak` crate, so Japanese line-start and line-end
  rules, and not breaking inside words or numbers, follow the Unicode classes
  rather than a hand-written table. A word longer than the line is broken
  between characters, and the count is reported.
- `--kinsoku normal` (the default) allows a line to start with a small kana or
  the prolonged sound mark (UAX #14 class CJ). `--kinsoku strict` does not, as
  in the strict rules of JLReq.
- `--hang` lets one closing comma or full stop (`、。，．` and their halfwidth
  forms) hang past the line width instead of being pushed to the next line
  with the preceding character.

`--vertical` sets the text top to bottom in columns that run right to left, and
`--wrap` becomes the column height. Line breaking, `--kinsoku` and `--hang` work
as in horizontal text, with punctuation hanging past the column foot, and
`--align left|center|right` aligns each column to the top, middle or bottom. Each character's
orientation comes from the Unicode `Vertical_Orientation` property (UAX #50):

| Value | With a `vert` glyph | Without one |
| --- | --- | --- |
| U | the `vert` glyph | upright |
| R | the `vert` glyph | turned a quarter clockwise |
| Tu | the `vert` glyph | upright, and counted |
| Tr | the `vert` glyph | turned a quarter clockwise, and counted |

Quarter turns are exact pixel substitutions. An upright character takes its
advance in width and the line height in height; a turned one takes the
reverse. `--tcy N` (vertical only) sets runs of up to N ASCII digits side by
side in one character slot, and falls back to upright digits, with a count,
when the run is wider than the column. Kerning is horizontal only and is not
applied in vertical text.

### BMFont output

`font bmfont` writes the AngelCode BMFont text format and RGBA page images.

```sh
pxsmith font bmfont out/misaki.pxfont.toml out/misaki.fnt
pxsmith font bmfont out/misaki.pxfont.toml out/ui.fnt --chars-file script.txt --page-size 128
```

- The output must end in `.fnt`. Pages are written next to it as
  `<stem>_0.png`, `<stem>_1.png`, and so on.
- Each glyph is trimmed to its ink box and shelf-packed, tallest first, with a
  one pixel gap into square pages of `--page-size` pixels (a power of two,
  default 256). The same input always gives the same output. `xoffset` and
  `yoffset` are measured from the glyph cell, so the cell geometry survives the
  trim; whitespace becomes a 0x0 glyph with only an advance.
- Single-colour fonts are written white on transparent, for tinting in the
  engine (for example with `modulate` in Godot). Colour fonts keep their
  colours.
- `[[kern]]` pairs become `kerning` lines. Vertical forms are not written,
  since the format has no vertical layout, and the number left out is
  reported.
- `--chars-file` limits the output to the characters in that file; a character
  the font lacks is an error. Kerning pairs are kept only when both characters
  are written.
- The font's `copyright` is printed at the end. The tool does not take on a
  font's licence obligations, but it does not drop the owner's notice either.

[`examples/godot-ui/`](../examples/godot-ui/) shows two such fonts in Godot 4.7
`Label` nodes. A capture there matches `font render` pixel for pixel, kerning
included.

### Rolling digits

`anim roll` turns one string of digits into another the way a score counter
does.

```sh
pxsmith anim roll out/misaki.pxfont.toml --from 0129 --to 0131 --frames 4 \
    out/score-roll.px.toml --palette palettes/sweetie-16.hex --ink 12
pxsmith anim ease out/score-roll.px.toml out/score-roll.gif --fps 60 --hold 1,1,2,3,4,1,1,2,3
```

- Each changed digit scrolls inside its own cell (as wide as the widest
  character that digit shows), so nothing crosses into the neighbouring digit.
  With the default `--direction up` the old digit leaves upward and the
  digits count up, wrapping from 9 to 0; `--direction down` reverses both.
- Each change takes `--frames` intervals. A digit that changes several times
  passes through the intermediate digits, and digits with fewer changes start
  later, so every digit lands together. `0129` to `0131` with `--frames 4`
  gives 9 frames: the ones digit goes 9, 0, 1 and the tens digit moves only
  during the second half.
- `--from` and `--to` must have the same number of characters (pad with zeros
  yourself). Characters that are not digits may appear but must not change.
- The first and last frames are key frames and match `font render` of the two
  strings; the rest are inbetweens. The frames carry no timing of their own:
  apply it with `anim ease`, as with the other `anim` commands.
- `--palette`, `--ink` and `--letter-spacing` work as in `font render`.

## UI parts

### 9-slice declarations

An L0 file may declare 9-slice borders in `[meta]`:

```toml
[meta]
format = 1
name = "window"
nine_slice = { left = 4, top = 4, right = 4, bottom = 4, stretch = "tile" }
```

`stretch` is `"stretch"` (the default), `"tile"` or `"tile_fit"`, the same three
modes as Godot's `AxisStretchMode`. Use `stretch_x` and `stretch_y` instead to
set the axes separately. The borders must leave at least one pixel of centre in
each direction. Files without `nine_slice` keep their canonical bytes, and the
L0 format version remains 1. `.aseprite` has no place for the declaration, so
`text import` warns that it is lost when the source L0 has one.

```sh
pxsmith nineslice preview examples/godot-ui/window.px.toml out/window.png --size 64x32 --size 120x48
```

`nineslice preview` stretches the first frame to each `--size WxH` and writes a
PNG. With one size the output name is used as given; with several, the size is
appended to the stem (`window-64x32.png`, `window-120x48.png`). The stretching
is a port of Godot 4.7's nine-patch mapping, evaluated in `f32` at pixel
centres, and Godot 4.7.2 `NinePatchRect` captures match it for all three modes
with zero differing pixels. When a tiled band's first and last pixels differ,
the command advises that the seam will show. A target smaller than the
borders is an error, as is an L0 file without `nine_slice`.

### Godot StyleBoxTexture export

`sheet pack` copies each input's `nine_slice` into its cell in the sheet
metadata, so an exporter can read it without the source files. `export godot`
then writes one `StyleBoxTexture` resource per 9-slice cell.

```sh
pxsmith sheet pack out/ui.png --input examples/godot-ui/window.px.toml \
    --input examples/sprites/hero_body.px.toml --meta out/ui.sheet.json
pxsmith export godot --sheet out/ui.sheet.json --out-dir out/godot --res-dir res://ui
```

- Each cell with `nine_slice` becomes `<cell name>.stylebox.tres` in
  `--out-dir`, in Godot 4.7's text resource format: the sheet texture as an
  `ExtResource` under `--res-dir` (which must start with `res://`), the
  texture margins, both axis stretch modes, and a `region_rect` taken from the
  cell's origin and the source image's own width and height. Using the cell
  rectangle instead would put the uniform grid's padding inside the 9-slice.
- No `uid` is written; Godot assigns one on import. No `.import` file is
  written either, so set nearest-neighbour filtering in the Godot project.
- Cells without `nine_slice`, and the Theme resource, are not written, and the
  command reports how many it left out. A palette-swap (format 2) sheet is
  exported with a warning that its texture holds palette indices and needs the
  shader.

In [`examples/godot-ui/`](../examples/godot-ui/), panels styled with the
exported resources in Godot 4.7.2 match the `nineslice preview` output for all
three stretch modes with zero differing pixels.

## Exporting

```sh
pxsmith sheet pack out/sheet.png --input a.px.toml --input b.px.toml --meta out/sheet.sheet.json
pxsmith export tiled map.json --tsx out/sheet.tsx --sheet out/sheet.sheet.json
```

`export godot` writes Godot `StyleBoxTexture` resources for 9-slice cells; see
[UI parts](#ui-parts). Godot terrain sets are not exported.

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

## Applying a palette

```sh
pxsmith palette apply walk.px.toml walk-dark.px.toml --palette palettes/dark.hex
pxsmith palette apply walk.aseprite walk-dark.aseprite \
  --palette palettes/dark.hex --dither ordered
```

`palette apply` processes every frame and layer. It preserves frame count,
durations and layer metadata for L0 and Aseprite; L0 also keeps its document and
frame names, `kind`, default and per-frame anchors, and other L0 metadata.
Fully transparent pixels stay transparent for `none`, `ordered` and `error`
dithering and do not receive diffusion error. If the target palette has no
transparent entry, one is appended to the output palette; only opaque pixels
are matched to target colours.

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
