# Changelog

**English** | [日本語](CHANGELOG.ja.md)

All crates in this workspace share one version number. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) as Cargo applies it to `0.x`
releases: a change in the minor number may break the public API.

## [0.2.0] - unreleased

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

[0.2.0]: https://github.com/akitenkrad/rs-pxsmith/compare/067a9b8...HEAD
[0.1.0]: https://github.com/akitenkrad/rs-pxsmith/tree/067a9b81a6efb517e6c47f24af8d0d8b836743bf
