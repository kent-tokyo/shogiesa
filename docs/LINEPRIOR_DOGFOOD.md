# lineprior dogfooding: is a historical move prior worth testing in Sekirei?

**Scope**: this is a measurement runbook. Nothing here changes Sekirei's search or integrates
`lineprior` into it — `scripts/lineprior_dogfood.sh` only wires `shogiesa lineprior export` →
`lineprior tune` → `lineprior eval` together and summarizes the result. `lineprior` itself is a
separate, domain-agnostic action-prior tool that is not built or vendored by this repo; you supply
its binary path.

## Why this question, not a new shogiesa feature

`shogiesa lineprior export` already produces the JSONL `lineprior` needs — that part was the easy,
already-shipped half. The open question is whether historical move priors extracted from real
games are actually useful as a Sekirei search signal, or just look plausible without being better
than no prior at all. Before touching Sekirei's search code, measure: export real games, run
`lineprior tune`/`lineprior eval` against them, and read coverage/fallback-rate/top-k-hit-rate/MRR.
Only if those numbers look good does prior-guided move ordering become worth prototyping inside
Sekirei.

## Prerequisites

- `cargo build --release` in this repo (or pass `--shogiesa "cargo run --quiet --release -p
  shogiesa-cli --"` to the script instead of a built-binary path).
- A `lineprior` binary, built separately — this repo has no opinion on how you build it and never
  adds it as a dependency.
- `jq`, used by the script to read manifest/report JSON fields into `report.md`.

## Running it

```bash
scripts/lineprior_dogfood.sh \
  --games ./games \
  --lineprior /path/to/lineprior \
  --out runs/lineprior-shogi-001 \
  --source teacher_v012 \
  --max-ply 80
```

This produces, under `--out`:

- `shogi_observations.jsonl` — `shogiesa lineprior export`'s output
- `export_manifest.json` — its `--manifest` output (record/sequence counts, outcome/tag
  distributions, `unknown_outcome_count`)
- `shogi_best_config.json`, `shogi_tune_report.json` — `lineprior tune`'s output
- `shogi_eval_report.json` — `lineprior eval`'s output
- `report.md` — the human-readable summary described below

## Reading `report.md`

The metrics table pulls straight from `lineprior eval`'s JSON: `coverage`, `fallback_rate`,
`top1_hit_rate`, `top3_hit_rate`, `top5_hit_rate`, `mrr`. **Weight `top5_hit_rate` and `mrr` over
`top1_hit_rate`.** The intended Sekirei use case is candidate-set move ordering — improving which
moves get searched first/deepest — not replacing search with a single predicted "best" move. A
prior that reliably gets the teacher's move into the top 5 (high `top5_hit_rate`, low average rank
via `mrr`) is useful for that even with a middling `top1_hit_rate`.

**If any row in the metrics table reads `n/a`**, that's not "no data" — it means the installed
`lineprior`'s JSON output uses different field names than the ones this script's `jq` expressions
expect (`coverage`/`fallback_rate`/`top1_hit_rate`/`top3_hit_rate`/`top5_hit_rate`/`mrr`; see the
caveat at the top of `scripts/lineprior_dogfood.sh`). By default the script still exits 0 in that
case, since a schema mismatch should stay non-fatal for exploratory runs — check
`shogi_eval_report.json` directly and fix the `jq` expressions in the report-generation step,
rather than assuming the run failed. Pass `--strict-report-fields` for a reproducible/logged
dogfood run you don't want to eyeball for `n/a` by hand: the script still writes `report.md`, but
exits non-zero (naming the missing fields) if any of the six required metrics came back `n/a`.

The Export section surfaces `export_manifest.json`'s counts directly, in particular
`unknown_outcome_count` — every move inside a KIF `変化` (variation) branch resolves to
`outcome: "unknown"` by design (a branch's own ending isn't the actual game's result; see
README.md's `lineprior export` section), so a high unknown count just means a variation-heavy
corpus, not a data-quality problem to fix.

`report.md`'s "Commands run" section is the literal invocation used, including the resolved
`--out` paths — enough to re-run by hand without re-deriving flags from this script or from memory.

## Deciding whether to go further

Only consider prior-guided move ordering inside Sekirei if, together:

- `top5_hit_rate` / `mrr` look good for the candidate-ordering use case above,
- `coverage` is sufficient (the prior actually has data for a meaningful fraction of positions
  Sekirei would search), and
- `fallback_rate` is reasonable — not so high that the prior rarely applies, not so low that it's
  suspiciously overconfident on sparse data.

If the numbers are middling, tune the *input* before touching Sekirei at all: adjust `--max-ply`
(narrower/wider ply range), `--source` (different or filtered game corpus), restrict to
opening-only positions, or re-run `lineprior tune`'s own confidence-mode/threshold sweep against a
different split. Only once measurement says this is worth it does Sekirei integration become the
next question — and that's a separate change, out of scope for this script and this repo.
