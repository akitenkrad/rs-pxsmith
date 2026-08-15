# How this was built

**English** | [日本語](engineering.ja.md)

[← Back to README](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md)

This document is a record rather than a manual. It is the distilled form of a
development log kept alongside the code, and it sets out the practices that
repeatedly paid off together with the specific mistakes that produced them.

Nearly every entry below exists because something went wrong first. The numbers
are real measurements, kept so that the claims can be checked rather than
believed. The measurements themselves live in
[`status.md`](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/status.md)
and
[`investigations/`](https://github.com/akitenkrad/rs-pxsmith/tree/main/docs/investigations),
while this page is concerned with how the work was done.

---

## 1. Measuring the ceiling before writing the feature

The cheapest feature is the one that measurement talks you out of.

When a feature needs a recogniser, it is often possible to build the thing that
no recogniser can beat and measure that instead. Such a construction is usually
brute force and usually disposable, but it settles the question with no
production code at all.

Discs are the clearest case. Jaggy detection fires on correctly drawn discs, and
the remaining valleys look like seams between two straight spans. Before writing
a seam recogniser, every way of extending a window on both sides of a valley was
tried exhaustively, which gives an upper bound no recogniser can exceed. The
result saves at most 32 of 64 cases and helps on real artwork in 1 case out of
105. The decisive finding was that a genuine defect made by shifting one step of
a disc produces the same run sequence as a correct disc edge. Two things that
must be told apart have the same shape, so the feature was never written.

The same procedure retired several other features before they were written.
`pxsmith gen image` was one of them: the design rejects any generated image whose
grid is non-uniform, and how often a diffusion model clears that bar depends on
the model, so the work has no readable ceiling and no point at which it could be
called finished. RotSprite was another, since no public specification exists and
the only public implementation notes that its own description is ambiguous. Once
`cleanedge` had been measured and found to pay off, there was nothing left for
the time to buy. Three lint variants were also dropped, each having fired on
artwork that was correct.

## 1a. Two behaviours were measured and deliberately left alone

Not every finding leads to a change. Where the fix would revise a numbered design
decision, or where the ceiling is too low to be worth it, the behaviour stays and
the number is recorded.

Jaggy detection fires on correctly drawn staircases. For a slope `a/b` a valley
exists exactly when `2·(b mod a) ≥ a`, which is a counting property rather than a
threshold, so sweeping cannot remove it. Rather than touch the detector,
`pxsmith smooth` was taught to decline to move pixels on spans that are digitally
straight. That brought the damage on clean artwork down from 88 pixels across 17
sprites to 60 across 13, and left discs, where it stops.

Discs are not protected, for the reason measured above. No recogniser can
separate a correct disc edge from a defect that produces the same run sequence.

## 2. A design document makes claims, and claims are sometimes wrong

Specification prose was checked against measurement wherever it asserted
something falsifiable, and it turned out to be wrong 10 times in one milestone
and 3 times in another.

Two failure shapes recurred. The first is that two procedures joined by "or"
often have different inputs. A projection section offered "rotate 45° then halve
the height" or "narrow to 0.866 then skew −30°", but these are not alternatives:
one lays a top-down view onto the floor while the other tips a side view onto a
wall. Implemented as interchangeable, the tool would have been silently wrong for
one of them.

The second is that a table and the procedure beside it can disagree. One
section's table said "2:1, used because 30° cannot be drawn", while the procedure
four lines later used `tan 30° = 0.577`. Counting on a scene with known ground
truth settled it, since at 2:1 the runs are one length whereas at 30° they split
into two.

One's own guesses fail in the same way. On one investigation both predictions
were wrong before a line was written: the damage was in discs rather than
staircases, and the local shape of a valley does not separate true from false at
all, since depth, neighbour difference, and position within the span had
identical distributions on both sides.

## 3. Building a scene where the truth is known

A false-positive rate measured on unlabelled artwork is not a false-positive
rate, because nobody can say whether a given dent was the artist's intent.

The alternative is to construct inputs whose correct answer is forced by
construction. Discs and rational-slope staircases have edges fully determined by
geometry, so every detection there is a false positive by definition. That
reframing turned "146 detections out of 24,714 runs, meaning unknown" into
"28.02% on isolated-run staircases and 0.00% when short runs are adjacent", and
then into a closed form: a valley exists exactly when `2·(b mod a) ≥ a`. Twenty
slopes agreed with the formula, eight of which were added after it was written.

The result also decides what to do about it. A counting property is not a
threshold, so sweeping cannot remove it, and the finding was recorded while the
detector was left alone.

## 4. Checking what the metric is actually measuring

This was the most expensive recurring mistake, and it happened four times.

"Negative-example capture: 53/69" is a detection number rather than a repair
number. An exemption added to the repair path does not change detection at all,
so by that metric any exemption reads as having no impact. What `smooth` could
actually repair was 31 of 69.

Lint cannot answer the question "is this the frame I asked for?" either. Using
blocking violations to judge whether a fast subpixel method produces usable
inbetweens gives 91.8%, whereas the right criterion, namely whether the
silhouette moved, gives 47.5%. Twenty-eight sprites moved their silhouette
without adding a single violation, because a shifted outline is not a broken
sprite but a correctly drawn different one. A checker asks whether the art is
valid, not whether it is what was wanted.

It is also worth confirming that the measuring instrument shares the tool's
judgement. After one rule was rewritten the numbers did not move at all, because
the measuring harness was still carrying its own stale copy of the decision.
Judgement now lives in exactly one function, which the harness calls.

## 5. A test existing does not mean it runs

On six separate occasions, code was found that could do nothing at all, protected
by tests that certified the emptiness as correct.

| What was empty | How it survived |
| --- | --- |
| An inflection-point rule reading the curvature field | Curvature never reverses sign inside a monotone span, so the exemption never applied. For three milestones the ideal single-valley shape was reported as a jaggy |
| A same-colour-neighbour helper searching 4-connected | Regions sharing an edge are already merged by fill, so it could never return anything. The original test fixed that behaviour as correct |
| A comment reading "read the error body if you can" | The HTTP client defaulted to turning 4xx into an error without the body, so the tool could say "HTTP 401" while the body said "API key is invalid." |
| A truncation path | The token limit was a constant, so reaching it required a response larger than any real one. Only the fake backend could get there |
| A whole lint rule, on 64 real sprites | It reported "0 violations" having never once been in a position to run |
| A dependency declared but never used | It would have been published as a dependency on a stranger's crate, silently |

Two habits follow from this. The first is to break a passing test on purpose in
order to confirm that it is load-bearing: flipping one layer-type bit and
disabling linked-cel resolution each make the round-trip comparison fail, which is
what makes its silence worth something. The second is to distinguish "did not
fire" from "could not run" in the tool's own output, since otherwise a quiet
report is indistinguishable from a disconnected one.

## 6. Confirming that a negative example is actually a defect

Six times, a rule was evaluated against a "defect" that was not one.

The clearest case involved a rule that counts valleys in a contour, measured with
negative examples made by pushing a pixel outward. An outward bump creates a
direction reversal, which splits the span, which leaves no valley on either side.
The rule was structurally incapable of seeing those inputs, and the measurement
read "8% capture".

Writing "check your negative examples" in the log three times did not prevent it
from happening again. What stopped it was counting mechanically whether each
negative example produced the feature the rule looks for, and using that count as
the denominator. With correct negatives the same rule captures 53 of 69.

## 7. Not measuring a varying thing once

Sweeping fields one at a time to find which one broke a request looked
conclusive, since removing the system prompt fixed it. On a second run the same
body passed and a different one failed. The variable was not a field at all but
time: responses under 60 seconds always succeeded, while three attempts over 60
seconds died at 60.07 seconds each. A control stream to another host survived 200
seconds, so the limit was on silence rather than on duration.

The same error recurred as an invented law. "32×32 takes 8.4 times as long as
16×16" was written from one sample of each, and a four-frame 16×16 request, which
has the same pixel count as 32×32, took one sixth of the time.

## 8. Running it end to end

Several defects were invisible to unit tests and appeared the first time the CLI
was driven from one end to the other.

`lint` continued advising "run `smooth` to fix this" after `smooth` had been
taught to leave those pixels alone, so the advice contradicted the behaviour. A
resample report said "opaque pixels 1024 → 1058" with no way to tell art from
padding, because a sprite with no declared transparent index has no "nothing" to
widen into. A subpixel command printed a population statistic on every run
regardless of whether that particular run had moved anything. And the manual
promised that `--dry-run` would show the assembled request, while what it showed
was four summary lines.

The packaging equivalent is `cargo publish --dry-run`, which recompiles the
packed tarball against the registry and therefore finds errors that cannot occur
in the workspace. Three of five crates failed the first time it was run, one of
them by silently resolving a path dependency to an unrelated crate of the same
name.

"End to end" also includes platforms one does not develop on. The L0 text format
split rows on `\n` and kept whatever came before it, so a file delivered with
CRLF line endings put a carriage return in every row and failed with "the
character `␍` is not in the [palette] map". L0 is meant to be edited by hand and
Windows git converts line endings on checkout, so this was reachable by any
Windows user while being invisible on macOS and Linux. The sibling `.hex` reader
had used `str::lines()` from the start and was never affected, so the entire
difference was one call site.

## 9. Reading the specification before writing about it

Prompt text describing the tool's own file format was written from memory and was
wrong, since it described an inline palette and a pixel array, neither of which
exists. A generation loop hides this perfectly, because a wrong prompt looks
exactly like a weak model.

The converse also paid off. Reading the refusal semantics first revealed that
fallbacks were routing refusals to another model, which is why a refusal never
arrived. That is a property of the request rather than of the classifier, and it
would have been invisible from the outside.

## 10. Reporting rather than prescribing, and saying what was not done

Where the tool cannot decide for the user, it reports and lets them decide. The
coverage report gives four thresholds instead of one, the projection command
requires the user to state which face is being laid down because guessing fails
silently, and `conform` refuses a non-uniform grid rather than guessing.

Where something was skipped, the tool says so and counts it. The round-trip test
reports how many independently authored files it saw and declares the requirement
open when that count is zero. A generation run keeps every failed attempt. A
feature that was deliberately not written is documented as such, so that it is
not rediscovered later as a gap.

## 11. Two implementations of one job is one too many

Whenever a second copy of a decision appeared, it drifted. The recipe format
therefore reads argument names and their order out of the command-line parser
rather than a hand-written table. Rotation, scaling, and projection all pass
through one mapping function, and grid judgement lives in one place that both the
tool and the harness call.

The corollary is that dead paths are deleted rather than kept. When responses
moved to streaming the batch parser became unreachable, and seven tests would
otherwise have gone on certifying it forever.

## 12. Not selecting a group by name

Pulling out a subset by matching on a label produced the wrong subset and the
conclusion came out backwards, since the group that was grabbed was the one that
must not fire. The safe procedure is to enumerate everything and then read.

Handed-down observations deserve the same suspicion. A note asserting "the shapes
are all `[2, 1, 2]`" turned out to describe 40 of 64 cases, the rest being
`(3,1,2)`, `(3,2,3)`, and `(5,1,2)`. It was a reading taken from the large
examples only.

---

## What this costs, and what it buys

This way of working is slow. Measuring a ceiling, building a ground-truth scene,
and breaking one's own tests all take time that produces no feature.

What it buys is that the numbers in this repository mean something. Where a
measurement came out badly the feature is not shipped and the number is written
down, so that the next person does not spend a week rediscovering that discs
cannot be separated from disc-shaped defects.
