# Sekirei gate evaluation: does `make-gate-openings` actually improve gating?

**Scope**: this is a runbook for evaluating Sekirei's own gating/match-runner infrastructure, not
a shogiesa feature. Nothing here is executed by shogiesa or verified in this repo's test suite —
`make-gate-openings` and its manifest/coverage output are already implemented and tested on the
shogiesa side; what's untested is whether a Sekirei strength gate run against its output is
actually *better* (lower-variance, less biased) than what Sekirei uses today. That question can
only be answered against a real Sekirei checkout and match-runner, which this repo doesn't have.

## Why this question, not a new shogiesa feature

shogiesa can already produce an opening suite (`make-gate-openings`) that is byte-for-byte
format-compatible with Sekirei's production `data/gate/openings_standard.sfen`. Producing the
file was the easy, already-shipped part. The open question is evaluation accuracy on Sekirei's
side: does using a shogiesa-built suite (instead of Sekirei's existing static file, or no opening
book at all) change gate outcomes in a way that matters — tighter Elo confidence intervals, less
opening-side bias, less domination by one source root — or does it just look different without
being better? Answering that requires running Sekirei's actual gate multiple times per arm and
comparing results, which is exactly the kind of thing `scripts/sekirei_dataset_ablation.sh`'s
`SEKIREI_GATE_CMD` hook already exists to wire up, just applied to opening-suite choice instead
of dataset-filtering choice.

## The four arms (A/B/C/D)

| Arm | Opening source | Notes |
|---|---|---|
| A | `startpos` only, no opening book | The no-book baseline — every game starts identically. |
| B | Sekirei's existing production `data/gate/openings_standard.sfen` | The current default; what any change is measured against. |
| C | `shogiesa make-gate-openings --input <positions.jsonl> --out c.sfen --count 100` | shogiesa-built suite, same rough size as typical gate books. |
| D | `shogiesa make-gate-openings --input <positions.jsonl> --out d.sfen --count 400` | Same as C, 4x the suite size — isolates "does more opening variety help" from "does a shogiesa-built suite help at all." |

`--input` for C/D should be a real, varied position corpus (e.g. an `extract`ed game archive),
not a small/synthetic sample — the whole point is testing realistic opening diversity.
`--min-ply`/`--max-ply` (defaults 8/unbounded) and `--seed` should be held fixed across every
repeat of a given arm, so re-running an arm changes only match-runner randomness, not the
opening suite itself. Record the exact `make-gate-openings` invocation (including `--seed`) and
its `--manifest` output alongside each arm's results, so a re-run is exactly reproducible.

## What to measure per arm

Run each arm's gate **multiple times** (not once — single-run numbers can't distinguish a real
effect from match-runner noise). For each arm, across its repeated runs:

- **Gate result variance run-to-run**: how much does the gate's win/draw/loss record (or
  whatever Sekirei's gate reports) vary between repeats of the *same* arm? A good opening suite
  should reduce this, not just change the mean.
- **Elo confidence-interval width**: narrower is better — it means the suite is giving the match
  a more stable, decisive signal rather than one dominated by opening-move luck.
- **Opening-side win-rate bias**: does one side (black/white) win disproportionately more across
  the suite? `make-gate-openings`'s output has no side-balance guarantee by construction (it
  quotas by root-diversity, not by side), so this is worth checking, not assuming.
- **Source-root dominance**: for C/D, does `make-gate-openings --manifest`'s own
  `distinct_roots_kept`/`max_root_share_in_any_bucket` (already reported) actually correlate with
  gate-result stability? A suite with one dominant source root defeats the purpose of using a
  suite instead of `startpos` at all.
- **Pass/fail decision stability**: if Sekirei's gate makes a binary ship/no-ship call, does that
  call flip between repeats of the same arm? An opening suite that reduces variance should make
  this call more consistent, not just move the numbers.

## How this slots into existing tooling

`scripts/sekirei_dataset_ablation.sh` already has the plumbing this needs — `SEKIREI_TRAIN_CMD`/
`SEKIREI_GATE_CMD` env-var hooks, per-arm output directories, a `report.md` summarizing results
across arms — but it's wired for *dataset*-arm comparison (baseline/tune-broad/tune-balanced/
tune-strict), not opening-suite comparison. Running this evaluation means either:

- Adapting a copy of that script with `ARMS=(startpos-only production-suite shogiesa-100
  shogiesa-400)` and each arm's "prep" step building/copying the right `.sfen` file instead of
  running `filter`, or
- Running each arm by hand against a real Sekirei checkout, using `SEKIREI_GATE_CMD`'s existing
  contract (`gate_result.json` per arm) as the shape to produce, so results are comparable
  side-by-side the same way dataset-ablation results are.

Either way, this is manual/external work against a real Sekirei checkout and match-runner — not
something this repo's test suite or CI can verify, since it depends on infrastructure (compute
for repeated match runs, Sekirei's actual gate implementation) that doesn't exist here.
