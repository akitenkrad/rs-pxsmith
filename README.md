# pxforge

A CLI and library for pixel-art asset pipelines — **a Makefile for pixel art**.

pxforge automates the generation, transformation, and validation of pixel-art
assets as a declarative pipeline. It has no interactive drawing UI: humans (or
generative models) produce the original artwork, and everything downstream —
derivation, reconciliation, and verification — runs as code.

> Status: **in development**. M0 (foundations), M1 (feedback loop), M1a (geometry),
> M2 (color and palette), M3 (cleanup and validation), and and M5 (pipeline) are complete;
> M4 (composition, tilesets, animation) is 18 of its 20 completion criteria — the
> sequence lint rules and `px atmos` remain. See `docs/status.md` for the details,
> including every claim that was measured rather than assumed.

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
| `px-lint` | The 18 implemented quality rules and their thresholds |
| `px-recipe` | The restricted expression language, dependency graph, step keys, and cache |
| `px` | The `px` command-line binary |
| `px-calib` | Measurement mouths. Every threshold in the tool was chosen with one of these |

Crates planned for later milestones: `px-export`, `px-gen`.

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

### Deriving artwork

```sh
# Derive shading from a silhouette (the source colors are discarded)
cargo run -p px -- shade hero.png hero.px.toml --base 8A6A4A --light dir:-0.6,0.8

# Normalize jaggies, add antialiasing, draw an outline
cargo run -p px -- smooth hero.px.toml smoothed.px.toml
cargo run -p px -- aa smoothed.px.toml aa.px.toml
cargo run -p px -- outline aa.px.toml outlined.px.toml --style tinted

# Animation: inbetweens, timing, cycles, smears, squash, subpixel, afterimages
cargo run -p px -- anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
cargo run -p px -- anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
cargo run -p px -- anim smear out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
cargo run -p px -- anim squash in.px.toml out.px.toml --amount -0.3
cargo run -p px -- anim subpixel in.px.toml out.px.toml --method tangent

# Check the result against hardware constraints (non-zero exit on violation)
cargo run -p px -- lint out.px.toml
cargo run -p px -- validate out.px.toml --target gb
```

### Recipes

A recipe is a TOML file. It has variables, a restricted expression language, and
a cartesian `for_each`; it has no loops, no function definitions, and no I/O, so
step keys resolve incrementally and builds stay deterministic.

```toml
[project]
format = 1

[vars]
seeds = ["hero", "slime"]

[[step]]
op = "shade"
input = "src/${s}.png"
output = "out/${s}.px.toml"
base = "8A6A4A"
light = "dir:-0.6,0.8"
for_each = { s = "${seeds}" }

[[step]]
op = "anim.squash"
input = "out/hero.px.toml"
output = "out/squashed.px.toml"
amount = -0.3
```

`op` maps one-to-one onto the CLI: `op = "anim.squash"` is `px anim squash`, and
the argument names and their order are read out of the command-line parser rather
than from a hand-written table.

```sh
cargo run -p px -- run build.toml --dry-run   # show the order, run nothing
cargo run -p px -- run build.toml --explain   # show each step key and its argv
cargo run -p px -- run build.toml --gc        # drop cache entries this recipe no longer uses

# Animated GIF of how one artefact came to be (its ancestry, in build order)
cargo run -p px -- run build.toml --progress how.gif --progress-of out/hero.px.toml

# Generate a recipe from external data (one [[step]] per row, so pairings survive)
cargo run -p px -- recipe expand template.toml build.toml --data rows.csv
```

The progress GIF is written with one local colour table per frame, so the colours
come out exactly as they went in — indices are `u8` and alpha is binary, which is
precisely what a GIF frame can hold, so no requantisation is needed.

Re-running an unchanged recipe restores everything from `.pxcache/`. On 128 steps
over 64 sprites this is 2.66 s cold against 0.09 s warm, and changing one input
rebuilds exactly the two steps that depend on it.

Sprites can also be embedded in Rust at compile time:

```rust
use px_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

Row-length mismatches become compile errors, and editing the referenced
palette triggers a rebuild.

## License

Undecided. Until then this repository is not published.
