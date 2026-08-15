# Generation

**English** | [日本語](generation.ja.md)

[← Back to README](../README.md)

`pxsmith gen prog` asks a language model for L0 text, then parses, lints, and — if
a blocking rule fires — sends the findings back and asks again. The checker is
the same `pxsmith lint` used everywhere else; nothing was written specially for
this.

```sh
export ANTHROPIC_API_KEY=...

pxsmith gen prog out/chest.px.toml --prompt "a wooden chest, seen head-on" \
    --palette 1a1c2c,566c86,8a6a4a,b13e53,f4f4f4 --size 16x16

pxsmith gen prog out/x.px.toml --prompt "a chest" \
    --palette 1a1c2c,f4f4f4 --size 8x8 --dry-run   # print the request, send nothing
```

`--dry-run` prints the assembled request body. The API key lives only in a
header, so printing the body leaks nothing — it is the one way to read what
would be sent without spending a call.

## The model cannot invent a colour

L0 is not self-contained: its palette is a *reference* to an external `.hex`
file, and the body holds only index characters. So the tool writes the `.hex`
first, and the model writes characters.

This is not a restriction so much as a guarantee. "Never introduce a colour
outside the palette" holds **structurally** rather than by inspection — there is
no way for the model to express one.

## What is verified, and what is not

The repair loop turns on the model's output, not on the transport. A refusal, a
truncated response, or a backend error ends the run instead of retrying: the same
request would hit the same wall. Only "could not parse" and "a blocking rule
fired" produce another attempt, and both can carry advice into the next request.

Frame sequences are checked too. Six inter-frame rules apply once there is more
than one frame, and the system prompt states them — a rule you fail someone for
without telling them is not a rule.

## No seed, and the provenance file says so

The model accepts no `temperature`, `top_p`, or seed. "Same request, same image"
therefore **does not hold**, and the provenance file records why instead of
carrying a seed field that would be a lie. What makes a build reproducible is the
cache plus committing the result — see [Recipes](recipes.md).

Provenance also records **the model that answered**, which is not always the model
that was asked: fallbacks are enabled, so another model may serve the request. The
tool reads the served name off the response rather than echoing what it sent.

## External calls are gated

Generation ops refuse to run from a recipe without `--allow-generate`, so a build
cannot quietly reach the network.
