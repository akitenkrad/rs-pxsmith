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

Two crates in the workspace are not published. `pxsmith-view` handles terminal
preview and reaches `ansi_colours` (LGPL-3.0-or-later) through `viuer`, and
`pxsmith`, the CLI, depends on it. Distributing a built binary would carry the
LGPL relinking obligation, so the binary is built from source instead.
`pxsmith-calib` is the measurement harness used to choose thresholds and is not
intended for consumption.

The library name keeps the underscore form, so imports read
`use pxsmith_core::…`.

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
