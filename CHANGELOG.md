# Changelog

**English** | [日本語](CHANGELOG.ja.md)

All crates in this workspace share one version number. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) as Cargo applies it to `0.x`
releases: a change in the minor number may break the public API.

## [Unreleased]

## [0.3.3] - 2026-11-15

### Fixed

- `--emit-palette` on `shade`, `aa` and `outline` no longer writes alpha-zero
  transparency slots as `000000` rows; the emitted `.hex` now matches the
  `.px.hex` written by the same run byte for byte. The same opaque-only rule now
  also applies to `palette extract`, the `.hex` that `text export` creates and
  the palette that `tileset import` creates. Genuine `000000` colours are kept
  (MYTASK-3590).
- `direction --reshade` now re-shades with the light given by `--light`,
  computed as `shade --light` does, instead of always using the default light of
  `--preset`. Without `--light` the preset default is still used. The core adds
  `expand_with_light`; the existing `expand` behaves as before (MYTASK-3591).

## [0.3.2] - 2026-11-15

### Fixed

- L0 `.px.hex` output now removes only alpha-zero transparency slots while
  preserving every opaque colour, including unused entries, in its original
  order. Repeated L0 transformations and flattened composition no longer append
  a transparency-derived `000000` entry (MYTASK-3581).
- Recipe keys and graph edges now include every operation-specific file input,
  including L0 palette references; input and output uses of fields such as
  `meta` are distinguished, side outputs (`emit_palette`, `sheet.pack` meta and
  palette atlas, an L0's `.px.hex`, and similar) count as outputs for edges,
  cache restore and the undeclared-file warning, and the key format is versioned
  away from old cache entries (MYTASK-3582).
- `palette convert` now selects `.hex` or GIMP Palette output from the output
  extension and rejects formats it cannot write (MYTASK-3583).
- GIMP Palette input now requires its magic header, reports malformed colour
  rows with line numbers and rejects an empty import (MYTASK-3584).
- Frame and palette loaders now report supported input formats and actionable
  PNG conversion commands; recipe progress capture indexes PNG output exactly
  and reports every skipped file with its reason (MYTASK-3585).
- Centroid-aligned tweening and smearing no longer escape the canvas because
  the exterior boundary of the outside distance field is handled correctly
  (MYTASK-3586).
- Direction and standalone lint now report rule 7 sample shortages per image,
  and `lint --light` makes the rule available outside direction expansion
  (MYTASK-3587).
- Direction and light-source documentation now matches the implemented mirror
  pairs and coordinate convention, while `run --dry-run` rejects unknown
  operations and fields (MYTASK-3588).

## [0.3.1] - 2026-11-14

### Fixed

- `compose` now flattens every part, in layer order, for single-layer L0, PNG
  and GIF output instead of silently keeping only the first layer in L0
  (MYTASK-3576). Aseprite output still preserves layers.
- `palette apply` now processes every frame and layer and preserves frame
  count, durations and L0 names, kinds and anchors (MYTASK-3577).
- `palette apply` now keeps transparent pixels transparent with every dither
  mode and appends transparency when the target palette lacks it
  (MYTASK-3578).
- `anim smear` now gives alignment-specific guidance when a smear remains
  disconnected after centroid alignment (MYTASK-3579).

## [0.3.0] - 2026-11-14

This release adds pixel fonts and UI parts, and breaks the public API of
`pxsmith-core` and `pxsmith-io` by adding fields to public structs. The L0 text
format stays at version 1, and every `.px.toml` file written by 0.2 reads
unchanged; an L0 file that uses the new `nine_slice` declaration cannot be read
by 0.2. Sheet metadata keeps formats 1 and 2, and a 0.2 reader ignores the new
`nine_slice` field in it.

### Breaking changes

- `pxsmith_io::l0::L0Meta` gains `nine_slice: Option<NineSlice>`. It implements
  `Default`, so struct literals can end in `..Default::default()`; otherwise
  add `nine_slice: None`.
- `pxsmith_core::sheet::SheetCell` gains `nine_slice: Option<NineSlice>`.
  `SheetCell` does not implement `Default`, so struct literals must add
  `nine_slice: None`.
- `pxsmith_io::gif::GifReport` gains `rounded_frames`, the number of frames
  whose delay was rounded. It implements `Default`.
- `CoreError` gains variants for 9-slice, Godot export and digit rolling, and
  `IoError` gains `Font` and variants for BDF and TrueType import. Both enums
  are `#[non_exhaustive]`, so exhaustive matches do not break.
- **Commands that write frames no longer treat every unknown extension as
  `.aseprite`.** `.aseprite` and `.ase` write Aseprite, `.px.toml` writes L0,
  `.gif` writes a GIF, and `.png` writes a PNG when there is exactly one frame.
  A `.png` with several frames, or any other extension, is now an error that
  names the accepted ones. Before, `anim tween ... out.png` silently wrote
  Aseprite data under a `.png` name.

### Added

- **Pixel font format.** `.pxfont.toml` holds a font as one TOML file: each
  glyph is drawn as a whole cell (advance × line height), single-colour with
  `#` and `.` by default or multi-colour with `mode = "color"` and an L0-style
  `[palette]`, with `variant = "vert"` glyphs for vertical text and `[[kern]]`
  pairs. The library side is `pxsmith_core::font` and
  `pxsmith_io::pxfont::PxFontDocument`.
- `pxsmith font import` reads BDF 2.1 (`pxsmith_io::bdf`) and TTF, OTF and TTC
  (`pxsmith_io::ttf`) at a given `--size`. A 1-bit embedded strike at that size
  is used as it is; otherwise outlines are rasterised by pixel-centre winding
  with no antialiasing (`--outline` forces this). JIS X 0208 BDF fonts are
  mapped through the bundled Unicode `JIS0208.TXT` table, GSUB `vert`/`vrt2`
  become vertical variants, and GPOS or `kern` pair kerning becomes
  `[[kern]]`. Glyphs that leave their cell are an error unless `--clip` is
  given; `--chars-file` limits the import, and `--face` selects a TTC face.
- `pxsmith font check` fails when a text uses characters the font lacks, and
  with `--vertical` when vertical forms are missing. It advises on glyphs with
  ink in the right-most column, glyphs with no ink, and advances narrower than
  the glyph.
- `pxsmith font render` writes text as L0, `.aseprite` or PNG, horizontally or
  with `--vertical` (columns right to left, orientation from UAX #50, quarter
  turns by exact pixel substitution, tate-chu-yoko with `--tcy`). It applies
  kerning, wraps with UAX #14 line breaking through `unicode-linebreak`, and
  offers `--kinsoku normal|strict`, `--hang`, `--align`, letter and line
  spacing, and `--missing`. The library side is
  `pxsmith_core::font_layout::layout_text` and `pxsmith_core::vertical`.
- `pxsmith font bmfont` writes the AngelCode BMFont text format and RGBA pages,
  shelf-packed deterministically, with kerning pairs. Single-colour fonts are
  white for tinting. The library side is `pxsmith_core::bmfont`.
- **9-slice.** L0 `[meta]` accepts an optional `nine_slice` table (borders,
  and `stretch`, `tile` or `tile_fit` per axis). `pxsmith nineslice preview`
  stretches it exactly as Godot 4.7 does and warns about visible tile seams.
  `pxsmith sheet pack` copies the declaration into the sheet metadata, and
  `text import` warns that `.aseprite` cannot hold it. The library side is
  `pxsmith_core::nineslice`.
- `pxsmith export godot` writes a Godot 4.7 `StyleBoxTexture` resource
  (`.tres`) for each 9-slice cell of a sheet, from the sheet metadata alone.
  The library side is `pxsmith_core::export::godot_styleboxes`.
- `pxsmith anim roll` rolls one string of digits into another, odometer
  style, with each digit scrolling inside its own cell. Timing is applied with
  `anim ease`. The library side is `pxsmith_core::roll`.
- **GIF output.** Every command that writes a sequence of frames now writes an
  animated GIF when the output ends in `.gif`: actual size, looping,
  transparency kept, colours mapped exactly per frame, and delays rounded to
  10 ms with the rounded frames reported.
- `examples/godot-ui/` shows BMFont labels, `NinePatchRect` and
  `StyleBoxTexture` panels in Godot 4.7.2, with captures that match
  `font render` and `nineslice preview` pixel for pixel.
- New dependencies: `ttf-parser` 0.25.1 (MIT OR Apache-2.0) in `pxsmith-io` and
  `unicode-linebreak` 0.1.5 (Apache-2.0) in `pxsmith-core`. `pxsmith-io` now
  ships the Unicode Consortium's `JIS0208.TXT` mapping table in `data/`, under
  the Unicode License v3 included beside it.

## [0.2.0] - 2026-11-14

This release breaks the public API of `pxsmith-core`. The L0 text format stays
at version 1, and every `.px.toml` file written by 0.1 still reads unchanged.

### Breaking changes

- **`pxsmith_core::compose::Part::anchors` is now `Anchors`** instead of
  `BTreeMap<String, IVec2>`. `Anchors::default` holds the positions shared by
  every frame and `Anchors::frames` holds per-frame overrides. Code that built
  the map directly can move it into `Anchors { default: map, frames: vec![] }`,
  which behaves as 0.1 did. Look positions up with `Anchors::get(frame, name)`.
- `compose::Placement` gains `offsets`, one position per composed frame.
  `offset` keeps its 0.1 meaning (the frame-zero position).
- `sheet::SheetCell` gains `palette` and `sheet::SheetDoc` gains `palettes`,
  both `None` in the format 1 metadata that 0.1 wrote. Struct literals of these
  types must add the fields.
- `CoreError::ComposeAnchorMissing` gains `frame`, and
  `CoreError::SheetVersion` loses `expected` because two formats are now
  accepted. `CoreError` is `#[non_exhaustive]`, so the new variants listed
  below do not break exhaustive matches.
- `IndexedCanvas::opaque_bbox` now returns the whole canvas when the canvas has
  no transparent index, rather than `None`. A canvas without a transparent
  index is fully opaque, and `None` was being read as "nothing to draw".

### Added

- **Per-frame anchors in L0.** A `[[frame]]` may override any anchor from
  `[meta]` with its own `anchors` table. Frames without one keep the
  byte-exact canonical form of 0.1. Composition resolves each part's anchor on
  the frame it is showing, so a static part can follow a moving body, and the
  canvas is sized from every frame so packed cells do not change size.
- `pxsmith anchor track` extracts the resolved anchor positions of every frame
  as JSON, and `pxsmith anchor apply` writes such a track into another L0 file.
- **Colour vision deficiency check.** `pxsmith palette cvd` simulates protan,
  deutan and tritan viewers (Machado, Oliveira and Fernandes 2009, applied in
  linear RGB) and reports colour pairs that fall below one OKLCh
  just-noticeable difference. Pairs named with `--pair` are blocking and make
  the command exit with status 1. The library side is `pxsmith_core::cvd`.
- **Palette-swap sheet output.** `pxsmith sheet pack --palette-swap` writes an
  index texture and a palette atlas instead of merging every input into one
  256-colour palette, with `--variant BASE.hex=VARIANT.hex` adding recoloured
  rows. The metadata is sheet format 2; format 1 remains the default.
  `pxsmith sheet resolve` performs the shader's lookup on the command line, and
  `examples/godot-palette-swap/` shows the same lookup in Godot 4.7. The
  library side is `pxsmith_core::sheet::{pack_palette_swap, resolve}`.

### Fixed

- `pxsmith quantize` no longer destroys transparency. A PNG with a transparent
  background came out fully opaque as `.png` and fully transparent as
  `.aseprite`. `--colors N` now counts opaque colours only.
- `pxsmith sheet pack` now treats the canvas transparent index as transparent
  when it points at an opaque palette colour, as Aseprite does. Before, such
  pixels were drawn opaque.
- Writing artwork that uses no transparency to `.aseprite` no longer loses
  every pixel. The writer now picks an unused index as the transparent index,
  adds one when every index is used, and reports an error when the palette is
  already full at 256 colours.

## [0.1.0] - 2026-08-15

First release on crates.io: `pxsmith-core`, `pxsmith-io`, `pxsmith-lint`,
`pxsmith-recipe`, `pxsmith-macro`, `pxsmith-gen`, `pxsmith-view` and the
`pxsmith` command line tool.

[0.3.3]: https://github.com/akitenkrad/rs-pxsmith/compare/8e08444...0005e48
[0.3.2]: https://github.com/akitenkrad/rs-pxsmith/compare/a7ead7f...8e08444
[0.3.1]: https://github.com/akitenkrad/rs-pxsmith/compare/3ef788d...a7ead7f
[0.3.0]: https://github.com/akitenkrad/rs-pxsmith/compare/98dccb5...3ef788d
[0.2.0]: https://github.com/akitenkrad/rs-pxsmith/compare/067a9b8...98dccb5
[0.1.0]: https://github.com/akitenkrad/rs-pxsmith/tree/067a9b81a6efb517e6c47f24af8d0d8b836743bf
