# pxforge

A CLI and library for pixel-art asset pipelines — **a Makefile for pixel art**.

pxforge automates the generation, transformation, and validation of pixel-art
assets as a declarative pipeline. It has no interactive drawing UI: humans (or
generative models) produce the original artwork, and everything downstream —
derivation, reconciliation, and verification — runs as code.

> Status: **early development**. Milestones M0 (foundations), M1 (feedback loop),
> and M1a (geometry foundations) are complete.
> See `docs/status.md` for what works today.

## Design

The design is written in Japanese and lives outside this repository, in the
author's Obsidian vault (`設計書/ドット絵CLI-Rust/`). Key decisions:

| # | Decision | Consequence |
| --- | --- | --- |
| 1 | Indexed color is a first-class citizen | Palette swaps, hardware-constraint checks, and tile equivalence all share one representation |
| 2 | Drawing primitives, but no input devices | Avoids the cost of a GUI editor while keeping procedural generation viable |
| 3 | A retained layer and a working layer, kept separate | Byte-exact `.aseprite` round-trip and simple algorithms at the same time |
| 4 | Shape and shading are separate; shading is always derived | Flips, tweens, and recolors never break, and palette escapes cannot occur |
| 5 | No scripting language in recipes | Step keys resolve incrementally, keeping incremental builds deterministic |
| 6 | A small set of geometry foundations underpins everything | Contour tracing, distance fields, run-length analysis, local grid estimation, and region labeling carry 6 still-image features, 3 animation features, and 18 lint rules |

## Workspace

| Crate | Contents |
| --- | --- |
| `px-core` | Data model, geometry foundations, and pure-function algorithms. No I/O |
| `px-io` | Retained layer (`Document`), `.aseprite`, palette, and L0 text I/O |
| `px-view` | Terminal preview and diffing. Inspection only, by construction |
| `px-macro` | The `pixels!` proc-macro for embedding sprites in Rust |
| `px` | The `px` command-line binary |

Crates planned for later milestones: `px-export`, `px-lint`, `px-gen`, `px-recipe`.

## Build

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

`cargo-make` tasks are defined in `Makefile.toml`:

```sh
cargo make format-all   # taplo + clippy + rustfmt
cargo make test
```

## Usage

```sh
# Check the terminal can show pixels accurately (Kitty / iTerm2 / Sixel)
cargo run -p px -- verify terminal

# Convert a sprite layer to editable text, and back
cargo run -p px -- text export sprite.aseprite hero.px.toml --palette pal.hex
cargo run -p px -- text import hero.px.toml sprite.aseprite

# Watch a file and redraw on save
cargo run -p px -- watch hero.px.toml --zoom 8

# Show which pixels changed between two sprites
cargo run -p px -- diff before.px.toml after.px.toml

# Inspect or convert palettes (.hex is the canonical format)
cargo run -p px -- palette info palettes/sweetie-16.hex
cargo run -p px -- palette convert input.gpl output.hex

# Check that reading and writing an .aseprite file is byte-exact
cargo run -p px -- verify roundtrip sprite.aseprite --via-frame
```

Sprites can also be embedded in Rust at compile time:

```rust
use px_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

Row-length mismatches become compile errors, and editing the referenced
palette triggers a rebuild.

## License

Undecided (milestone M5). Until then this repository is not published.
