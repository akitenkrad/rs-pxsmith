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
pipeline. It has no drawing UI: a human — or a generative model — produces the
original artwork, and everything downstream runs as code. Shading is derived
from silhouettes rather than painted, inbetweens are computed, tilesets are cut
and deduplicated, and the result is checked against 27 quality rules before it
ships.

Colour is indexed end to end. Every transform is a choice among indices that
already exist, so **a palette escape is structurally impossible** rather than
merely checked for.

Every threshold in this tool was chosen by measuring something on real artwork,
and the measurements are kept so the numbers can be reproduced rather than
believed.

## Install

The library crates are on crates.io:

```sh
cargo add pxsmith-core pxsmith-io pxsmith-lint
```

The `pxsmith` command is **not** published, because it statically links
`ansi_colours` (LGPL-3.0-or-later) through `viuer`. Build it from source:

```sh
cargo install --git https://github.com/akitenkrad/rs-pxsmith pxsmith
```

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

`lint` distinguishes a rule that did not fire from a rule that *could not run*,
and says which happened. A quiet report is not evidence of a clean sprite unless
the check was in a position to fail.

## Documentation

| | |
| --- | --- |
| [Command line](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/cli.md) | Every subcommand, with the flags that matter |
| [Recipes](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/recipes.md) | The declarative build format and its cache |
| [Generation](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/generation.md) | Asking a language model for artwork, and verifying what comes back |
| [Library](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/library.md) | Using the crates from Rust, and the `pixels!` macro |
| [Architecture](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/architecture.md) | The crate split, the design decisions, and how the thresholds were chosen |
| [How this was built](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/engineering.md) | The development philosophy, and the mistakes that produced it |

The engineering record lives in [`docs/status.md`](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/status.md) and
[`docs/investigations/`](https://github.com/akitenkrad/rs-pxsmith/tree/main/docs/investigations):
what was measured, and what the numbers were.

## License

Licensed under either of [Apache License 2.0](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-APACHE) or
[MIT license](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-MIT) at your option — the usual dual licence for Rust
crates, so this code can be used from either side of that ecosystem.

`crates/pxsmith-core/src/cleanedge.rs` is a port of the cleanEdge shader by
torcado, used under its own terms; see [NOTICE](https://github.com/akitenkrad/rs-pxsmith/blob/main/NOTICE) for the attribution it
requires.

Test material under `testdata/` is CC0 or MIT with the attribution recorded in
`testdata/SOURCES.md`. Material that cannot be redistributed is not committed.
