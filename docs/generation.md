# Generation

**English** | [日本語](generation.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

`pxsmith gen prog` asks a language model for L0 text, then parses it, runs the
lint, and, if a blocking rule fires, sends the findings back and asks again. The
checker is the same `pxsmith lint` used everywhere else, and nothing was written
specially for generation.

```sh
export ANTHROPIC_API_KEY=...

pxsmith gen prog out/chest.px.toml --prompt "a wooden chest, seen head-on" \
    --palette 1a1c2c,566c86,8a6a4a,b13e53,f4f4f4 --size 16x16

pxsmith gen prog out/x.px.toml --prompt "a chest" \
    --palette 1a1c2c,f4f4f4 --size 8x8 --dry-run   # print the request, send nothing
```

`--dry-run` prints the assembled request body. The API key lives only in a
header, so printing the body leaks nothing, and this is the one way to read what
would be sent without spending a call.

## The model cannot invent a colour

L0 is not a self-contained format. Its palette is a reference to an external
`.hex` file, and the body holds only index characters. The tool therefore writes
the `.hex` first, and the model writes characters.

This is less a restriction than a guarantee. The property "never introduce a
colour outside the palette" holds structurally rather than by inspection, because
the model has no way to express such a colour.

## What is verified, and what is not

The repair loop turns on the model's output rather than on the transport. A
refusal, a truncated response, or a backend error ends the run instead of
retrying, since the same request would meet the same wall. The loop runs only for
"could not parse" and "a blocking rule fired", and both of those can carry advice
into the next request.

Frame sequences are checked as well. Six inter-frame rules apply once there is
more than one frame, and the system prompt states them, because a rule someone is
failed by without being told is not a rule.

## No seed, and the provenance file says so

The model accepts no `temperature`, `top_p`, or seed, so the property "same
request, same image" does not hold. Rather than carry a seed field that would be
a lie, the provenance file records why no such field exists. What makes a build
reproducible is the cache together with committing the result, as described in
[Recipes](recipes.md).

Provenance also records the model that answered, which is not always the model
that was asked. Fallbacks are enabled, so another model may serve the request,
and the tool reads the served name off the response rather than echoing what it
sent.

## External calls are gated

Generation ops refuse to run from a recipe without `--allow-generate`, so a build
cannot quietly reach the network.
