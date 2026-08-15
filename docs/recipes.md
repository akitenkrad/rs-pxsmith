# Recipes

**English** | [日本語](recipes.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

A recipe is a TOML file. It provides variables, a restricted expression language,
and a cartesian `for_each`, but it has no loops, no function definitions, and no
I/O. The restriction is deliberate, because it lets step keys resolve
incrementally, so an unchanged step can be known to be unchanged without running
it.

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

`op` corresponds one-to-one with the CLI, so `op = "anim.squash"` means
`pxsmith anim squash`. The argument names and their order are read out of the
command-line parser rather than from a hand-written table, since a second table
would eventually drift while reading the parser cannot.

## Running

```sh
pxsmith run build.toml --dry-run   # show the order, run nothing
pxsmith run build.toml --explain   # show each step key and its argv
pxsmith run build.toml --gc        # drop cache entries this recipe no longer uses

# Animated GIF of how one artefact came to be (its ancestry, in build order)
pxsmith run build.toml --progress how.gif --progress-of out/hero.px.toml

# Generate a recipe from external data (one [[step]] per row, so pairings survive)
pxsmith recipe expand template.toml build.toml --data rows.csv
```

The progress GIF is written with one local colour table per frame, so the colours
come out exactly as they went in. Indices are `u8` and alpha is binary, which is
precisely what a GIF frame can hold, so no requantisation is needed.

## The cache

Re-running an unchanged recipe restores everything from `.pxcache/`. Across 128
steps over 64 sprites this takes 2.66 seconds cold and 0.09 seconds warm, and
changing one input rebuilds exactly the two steps that depend on it.

The cache is also what makes a build containing generated artwork reproducible.
The generation step itself is not deterministic, because the model accepts no
seed. What makes the build repeatable is that the result is cached and committed,
not that the model would answer the same way twice. See
[Generation](generation.md) for the details.

## Determinism

Builds are byte-identical across thread counts, and changing `RAYON_NUM_THREADS`
does not change any output. This is confirmed by a test rather than asserted.
