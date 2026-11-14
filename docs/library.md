# Library

**English** | [日本語](library.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

## Crates

| Crate | Contents |
| --- | --- |
| [`pxsmith-core`](https://crates.io/crates/pxsmith-core) | Data model, geometry foundations, and pure-function algorithms. No I/O |
| [`pxsmith-io`](https://crates.io/crates/pxsmith-io) | Retained layer (`Document`), `.aseprite`, palette, and L0 text I/O |
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
pxsmith-view = { version = "0.2", default-features = false }
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

## Embedding sprites at compile time

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

Row-length mismatches become compile errors, and editing the referenced palette
triggers a rebuild. The macro tracks the `.hex` file, so a palette change cannot
be something you forget to rebuild for.

## Why there is no export crate

Export targets such as Tiled, sprite sheets, and canonical JSON are output
adapters with no algorithms of their own. They therefore sit in `pxsmith-core`
beside the data they serialise and are wired up in the CLI. A crate boundary
there would separate a serialiser from the type it serialises for no gain.
