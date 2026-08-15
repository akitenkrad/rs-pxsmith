# Architecture

**English** | [日本語](architecture.ja.md)

[← Back to README](../README.md)

## Design decisions

| # | Decision | Consequence |
| --- | --- | --- |
| 1 | Indexed colour is a first-class citizen | Palette swaps, hardware-constraint checks, and tile equivalence all share one representation |
| 2 | Drawing primitives, but no input devices | Avoids the cost of a GUI editor while keeping procedural generation viable |
| 3 | A retained layer and a working layer, kept separate | Byte-exact `.aseprite` round-trip and simple algorithms at the same time |
| 4 | Shape and shading are separate; shading is always derived | Flips, tweens, and recolours never break, and palette escapes cannot occur |
| 5 | No scripting language in recipes | Step keys resolve incrementally, keeping incremental builds deterministic |
| 6 | A small set of geometry foundations underpins everything | Contour tracing, distance fields, run-length analysis, local grid estimation, and region labelling carry 6 still-image features, 3 animation features, and 18 of the 27 lint rules |

Decision 6 was tested before it was relied on. Building the foundations first
surfaced two design holes that per-feature implementations would have shipped
without noticing: a monotone run needs to be cut where the walk turns twice, not
merely where the sign flips; and inflection points cannot be read from run
lengths alone — they need the curvature field.

## How thresholds were chosen

Every threshold came from a measurement on real artwork, and the measuring mouths
are kept in `pxsmith-calib` so the numbers can be reproduced rather than
believed. The practice that mattered most:

- **Measure the ceiling before writing the feature.** Building the thing no
  recogniser can beat settles a question with zero lines of production code.
- **Build a scene with ground truth.** A false-positive rate over unlabelled
  artwork is not a false-positive rate — nobody can say whether a given dent was
  the artist's intent. Discs and rational-slope staircases have edges that are
  fully determined by geometry, so every detection there is a false positive by
  definition.
- **Check what a metric actually measures.** More than once a number moved for a
  reason unrelated to the change being evaluated.
- **Confirm that a negative example is actually a defect.** A rule that counts
  valleys cannot be evaluated with a negative example that produces no valley.
- **Run it end to end.** Several defects were invisible to unit tests and
  appeared the first time the CLI was driven from one end to the other.

## Verification

`.aseprite` files round-trip byte for byte across the 19 official test sprites,
including tilemaps, linked cels, groups, slices, tags, and user-data properties.

Byte equality is not proof of understanding, though: an unknown chunk carried
around as an opaque blob round-trips perfectly while being misread. So a
**second parser was written from the file-format specification alone**, without
looking at the first, and the two are compared on canvas size, frame count,
per-frame duration, layer order and kind and name, which layers are groups, the
palette, the transparent index, and which frames have cels. All 19 agree.

That test was then broken on purpose to confirm it was load-bearing — flipping
one layer-type bit, and ignoring linked-cel resolution, each make it fail.

What the corpus does **not** cover is counted rather than guessed: no `cel extra`,
no raw cels, no old palette chunks; and only one value ever appears for blend
mode, group depth, and colour profile. The corpus is also not independent — it
comes from the `aseprite-io` fixtures — so the round-trip test reports how many
independently authored files it saw, and says the requirement is not closed when
that count is zero.
