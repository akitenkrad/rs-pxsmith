# Library

**English** | [日本語](library.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

## Crates

| Crate | Contents |
| --- | --- |
| [`pxsmith-core`](https://crates.io/crates/pxsmith-core) | Data model, geometry foundations, and pure-function algorithms. No I/O |
| [`pxsmith-io`](https://crates.io/crates/pxsmith-io) | Retained layer (`Document`), `.aseprite`, palette, L0 text, and font I/O (`.pxfont.toml`, BDF, TTF/OTF/TTC) |
| [`pxsmith-lint`](https://crates.io/crates/pxsmith-lint) | The 27 quality rules and their thresholds: 21 on a single canvas, 6 across a frame sequence |
| [`pxsmith-recipe`](https://crates.io/crates/pxsmith-recipe) | The restricted expression language, dependency graph, step keys, and cache |
| [`pxsmith-macro`](https://crates.io/crates/pxsmith-macro) | The `pixels!` proc-macro for embedding sprites in Rust |
| [`pxsmith-gen`](https://crates.io/crates/pxsmith-gen) | The generation loop: request, provenance, and the verify-and-repair cycle |

Two further crates are published alongside these. `pxsmith-view` provides
terminal preview, and `pxsmith` is the command line tool, installed with
`cargo install pxsmith`. Only `pxsmith-calib`, the measurement harness used to
choose thresholds, stays unpublished, since it is not intended for consumption.

`pxsmith-view` is the one crate that reaches an LGPL dependency, and it does so
through a single feature. `term::detect` and `term::show` use `viuer`, which
pulls in `ansi_colours` (LGPL-3.0-or-later), and both sit behind the default
`terminal` feature. Taking the crate with `--no-default-features` removes `viuer`
and `ansi_colours` from the dependency tree entirely, while `render`, `diff`,
`onion` and `TerminalKind` remain available.

```toml
pxsmith-view = { version = "0.3", default-features = false }
```

The library name keeps the underscore form, so imports read
`use pxsmith_core::…`.

## Per-frame composition anchors

`pxsmith_core::compose::Part` stores anchors in `Anchors` rather than in a
single map. `Anchors::default` contains the positions shared by every frame,
and `Anchors::frames` contains per-frame overrides. An empty `frames` vector
means every frame uses `default`; otherwise its length must equal
`Part::frames.len()`, or `compose` returns `ComposeAnchorFrameCount`.

`Anchors::get(frame, name)` resolves the frame override before the default, and
`Anchors::names()` returns the sorted union of names from both levels. During
composition, each part's selected animation frame supplies its anchor. The
resulting `Placement::offset` remains the frame-zero position for callers that
only need the previous API meaning, while `Placement::offsets` contains one
position per composed frame.

The expanded canvas is computed once from every per-frame placement. This keeps
all output frames the same size even when an anchor moves; `ComposeOptions::clip`
continues to use the base part's canvas instead.

## Fonts and text layout

`pxsmith_core::font` is the font model, with no I/O. A `Font` holds `FontMeta`
(line height, baseline, default advance, `FontMode::Mono` or `FontMode::Color`
with its `FontPalette`, and the optional source and copyright), the glyphs, and
the `Kern` pairs. Each `Glyph` is one character, an optional `variant` such as
`"vert"`, an optional `advance`, and an `IndexedCanvas` covering the whole cell,
so its height is always the line height. `Glyph::effective_advance` falls back
to the width of the glyph's canvas, and to the font's default advance only when
that width is zero. A single-colour glyph uses index 0 for
transparent and 1 for ink. `Font::validate` checks these invariants and
`Font::canonicalize` puts glyphs and kerning pairs into canonical order.

The file format lives in `pxsmith-io`. `pxfont::PxFontDocument` reads and
writes `.pxfont.toml` (`read`, `parse`, `write`, `to_toml`) and converts to and
from the model with `to_font` and `from_font`; the canonical form round-trips
byte for byte. `bdf::import_bdf` takes BDF 2.1 text and `ttf::import_ttf` takes
TrueType or OpenType data. Both return the `Font` together with a report of
what was imported, skipped, clipped and, for OpenType, which glyph path was
taken. Their options (`BdfImportOptions`, `TtfImportOptions`) mirror the
`font import` flags.

`font_layout::layout_text(font, text, options)` lays text out and returns a
`FontLayoutResult`: an `IndexedCanvas` in the font's own indices with a
transparent background, and a `FontLayoutReport` of lines, substitutions,
forced breaks, hanging punctuation and vertical fallbacks. Mapping the indices
to an output palette is left to the caller, which is how the CLI applies
`--ink`. `FontLayoutOptions::default()` is horizontal, unwrapped text with
kerning on and `Kinsoku::Normal`; `WritingDirection::Vertical` selects vertical
text, and `tcy` takes effect only there. A character the font lacks returns
`FontLayoutError::MissingGlyphs` unless `missing` names a replacement.

`vertical::vertical_orientation(code_point)` returns the Unicode
`Vertical_Orientation` value (`U`, `R`, `Tu` or `Tr`) from a table generated
from Unicode 18.0.0's `VerticalOrientation.txt`. Layout and `font check` share
this one table.

`bmfont::build_bmfont(font, options)` produces the AngelCode BMFont text
descriptor and the RGBA page canvases in memory, without writing files.
`BmFontOptions::new(page_size, page_file_stem)` sets the page size and the
stem used in the descriptor's page names; `with_characters` limits the output
and `with_palette` supplies the colours a colour font needs
(`BmFontError::MissingPalette` without it).

`roll::roll(font, from, to, options)` returns the frames of `anim roll` as
`IndexedCanvas` values in the font's indices. `RollOptions` sets the number of
intervals per change, the `RollDirection` and the letter spacing.

## 9-slice and Godot export

`nineslice::NineSlice` holds the four borders and an `AxisStretch` per axis
(`Stretch`, `Tile` or `TileFit`). It deserialises either a shared `stretch` or
`stretch_x` and `stretch_y`, and serialises both axes explicitly, which is the
form the sheet metadata uses. `validate_source` requires at least one pixel of
centre in the source, and `validate_target` requires the borders to fit the
target. `render_nine_slice(source, spec, width, height)` stretches an
`IndexedCanvas` exactly as Godot 4.7 does, and `visible_seams` returns the axes
whose tiled band has differing end pixels.

`export::godot_styleboxes(sheet, res_dir)` turns the 9-slice cells of a
`SheetDoc` into `GodotStyleboxFile` values (a file name and the `.tres` text)
inside a `GodotStyleboxExport`, which also counts the cells it skipped and
flags a palette-swap sheet. It writes nothing to disk.

Two public structs gained a field for this, which breaks code that builds them
with struct literals. `pxsmith_io::l0::L0Meta` has
`nine_slice: Option<NineSlice>`; it implements `Default`, so
`..Default::default()` covers the new field. `pxsmith_core::sheet::SheetCell`
has `nine_slice: Option<NineSlice>` and does not implement `Default`, so
literals must add `nine_slice: None`. Both fields are omitted from the serialised form when
`None`, so existing L0 files and sheet metadata are unchanged.

## Embedding sprites at compile time

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

Row-length mismatches become compile errors, and editing the referenced palette
triggers a rebuild. The macro tracks the `.hex` file, so a palette change cannot
be something you forget to rebuild for.

## Why there is no export crate

Export targets such as Tiled, Godot StyleBox resources, sprite sheets, and
canonical JSON are output adapters with no algorithms of their own. They therefore sit in `pxsmith-core`
beside the data they serialise and are wired up in the CLI. A crate boundary
there would separate a serialiser from the type it serialises for no gain.
