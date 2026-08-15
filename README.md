<p align="center">
  <img src="https://raw.githubusercontent.com/akitenkrad/rs-pxsmith/main/docs/assets/logo.png" width="180" alt="pxsmith">
</p>

<h1 align="center">pxsmith</h1>

<p align="center"><em>A Makefile for pixel art.</em></p>

<!-- Restore after `cargo publish --workspace`:
  <a href="https://crates.io/crates/pxsmith-core"><img src="https://img.shields.io/crates/v/pxsmith-core.svg" alt="crates.io"></a>
  <a href="https://docs.rs/pxsmith-core"><img src="https://docs.rs/pxsmith-core/badge.svg" alt="docs.rs"></a>
-->
<p align="center">
  <a href="https://github.com/akitenkrad/rs-pxsmith/actions/workflows/ci.yml"><img src="https://github.com/akitenkrad/rs-pxsmith/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License">
  <img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust 2024">
</p>

**English** | [日本語](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

---

pxsmith derives, reconciles, and verifies pixel-art assets as a declarative
pipeline. It provides no drawing interface, because the original artwork is
expected to come from a person or from a generative model. Everything downstream
of that runs as code: shading is derived from silhouettes rather than painted,
inbetweens are computed, tilesets are cut and deduplicated, and the finished
asset is checked against 27 quality rules before it ships.

Colour is indexed from end to end. Every transform is a choice among indices that
already exist, so a colour outside the palette cannot appear. This is a
structural property rather than something the tool checks for afterwards.

Every threshold in this tool was chosen by measuring something on real artwork,
and the measuring harnesses are kept in the repository so that the numbers can be
reproduced rather than believed.

## Install

The library crates are on crates.io.

```sh
cargo add pxsmith-core pxsmith-io pxsmith-lint
```

The command line tool installs from the same registry.

```sh
cargo install pxsmith
```

`cargo install` builds on your own machine, which matters because the terminal
preview reaches `ansi_colours` (LGPL-3.0-or-later) through `viuer`. No prebuilt
binaries are distributed. Library users who want nothing to do with that
dependency can take `pxsmith-view` with `--no-default-features`, which removes it
from the tree entirely.

## Quick start

```sh
# Turn a sprite layer into editable text, and back
pxsmith text export sprite.aseprite hero.px.toml --palette pal.hex
pxsmith text import hero.px.toml sprite.aseprite

# Derive shading from a silhouette, then check the result
pxsmith shade hero.png hero.px.toml --base 8A6A4A --light dir:-0.6,0.8
pxsmith lint hero.px.toml

# Redraw in the terminal on every save
pxsmith watch hero.px.toml --zoom 8
```

`lint` reports a rule that did not fire separately from a rule that could not
run, because a quiet report is evidence of a clean sprite only when the check was
in a position to fail.

## Documentation

| | |
| --- | --- |
| [Command line](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/cli.md) | Every subcommand, with the arguments that matter |
| [Recipes](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/recipes.md) | The declarative build format and its cache |
| [Generation](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/generation.md) | Asking a language model for artwork, and verifying what comes back |
| [Library](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/library.md) | Using the crates from Rust, and the `pixels!` macro |
| [Architecture](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/architecture.md) | The crate split, the design decisions, and how the thresholds were chosen |
| [How this was built](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/engineering.md) | The development philosophy, and the mistakes that produced it |

The record of the measurements themselves is in
[`docs/status.md`](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/status.md)
and
[`docs/investigations/`](https://github.com/akitenkrad/rs-pxsmith/tree/main/docs/investigations),
which set out what was measured and what the numbers were.

## License

This project is available under either [Apache License 2.0](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-MIT),
at your option. This is the dual licence customary for Rust crates, and it is
used here so that the code can be consumed from either side of that ecosystem.

`crates/pxsmith-core/src/cleanedge.rs` is a port of the cleanEdge shader by
torcado and is used under its own terms. The attribution it requires is recorded
in [NOTICE](https://github.com/akitenkrad/rs-pxsmith/blob/main/NOTICE).

Test material under `testdata/` is CC0 or MIT, and its provenance is recorded in
`testdata/SOURCES.md`. Material that cannot be redistributed is not committed.
