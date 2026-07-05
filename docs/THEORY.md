# What shogiesa's numbers mean (and don't)

shogiesa is a data generation/validation/labeling/selection tool for Shogi training positions —
not a Shogi engine, not an NNUE trainer (see the top-level README). Every quality signal it
produces comes straight out of USI engine search output. None of them are calibrated
probabilities, and no logistic/Elo conversion exists anywhere in this codebase. This document is
the reference for what each one actually is, so a future reader doesn't misread a proxy as a
probability.

## `score.cp` is not a win probability

`Observation.score` is either `{"kind":"cp","value":N}` or `{"kind":"mate","moves":N}`. The `cp`
value is a raw USI engine evaluation in centipawns — nothing more. USI's `info score cp` convention
is **side-to-move-relative**: positive means "good for whoever is on move," and `shogiesa-usi`
records that raw value with no sign-flipping. `Observation.score_perspective`
(`ScorePerspective::SideToMove` or `::Black`) makes this convention explicit in the schema instead
of leaving it an undocumented assumption; `label` always produces `SideToMove`. When a consumer
needs "is Black winning" regardless of whose turn it was, it calls
`shogiesa_core::cp_from_black_perspective(cp, perspective, side_to_move)` — the one place that
sign-flip is centralized (used by `filter --eval-min/--eval-max`, `balance --by eval-bucket`,
`report`'s eval histograms, and `audit`'s `score_error_cp`).

None of this makes `cp` a probability. There is no function anywhere in shogiesa that converts a
`cp` value into "P(win)". A cp value of +800 isn't "80% win chance" — it's whatever the specific
engine that produced it happened to output at that search depth, in that position, under that
network. Different engines, different networks, and different depths are not directly comparable
on the same cp scale.

## `policy_margin_cp` is a MultiPV margin, not a confidence score

`Observation.policy_margin_cp` (populated only when `label --multipv N` with N≥2 was used) is
`score_cp(bestmove) − score_cp(runner_up)` — the gap between the engine's top choice and its
second-best line at the same search. It answers "how much better did the engine think its best
move was than its next-best alternative," which correlates with position sharpness, but it is not
a probability that the bestmove is actually correct, and it says nothing about whether a *third*
untried move might be better than both. `None` when MultiPV wasn't used, either score was a mate
score, or the runner-up's score was a lowerbound/upperbound rather than a confirmed evaluation
(`filter --min-policy-margin-cp`/`--require-policy-margin` gate on it; `calibrate
--sweep-policy-margin` measures what different thresholds actually do to coverage on your data).

## `score_swing_cp` is a cross-depth instability proxy

`shogiesa_core::score_swing()` computes max-minus-min across a record's cp-scored observations —
typically the same engine searched to different depths (`label --depths a,b,c`). A large swing
means the evaluation kept changing as search got deeper, which is a signal that the position is
tactically unstable or the search hadn't converged, not a direct measure of "how wrong" any single
observation is. It says nothing about whether the *deepest* observation is itself reliable — that's
a separate question `audit` addresses by comparing against an even-deeper teacher depth.

## `bestmove_agreement` is a teacher-consensus proxy, not a correctness check

`shogiesa_core::bestmove_agreement()`/`engine_bestmove_agreement()` check whether multiple
observations (across depths, or across distinct engines) picked the same move, excluding
`resign`/`win`/`none` tokens from the comparison via `effective_bestmove_kind()` — one engine
giving up isn't an opinion about which move is best, so it's neither agreement nor disagreement.
Agreement between two searches is evidence the position isn't ambiguous *to those searches*; it
is not proof the agreed-upon move is actually best. Two engines (or two depths of one engine) can
agree and both be wrong, especially in the endgame phases where search horizons dominate.

## `QualityDecision.score` is a transparent gate-passthrough readout, not a confidence value

This is the single most important thing to get right, and the one most likely to be misread.
`evaluate_quality()`'s `QualityDecision.score` is computed as, literally:

```rust
let score = if configured_gates == 0 {
    1.0
} else {
    1.0 - reasons.len() as f32 / configured_gates as f32
};
```

That is: the fraction of *whichever gates the caller configured* that this record passed. It is
**not** an independently-trained or weighted confidence value, it does not estimate "P(this
position is good training data)", and its meaning changes depending on which `QualityConfig` fields
were turned on — a record scoring `0.75` under one config and `0.75` under a different config are
not comparable to each other. A record's `score` says "3 of the 4 gates you asked about passed,"
nothing more. `select --strategy uncertain` reuses this score directly as a ranking key precisely
*because* it's transparent and reproducible, not because it's a calibrated probability — ranking by
"fraction of configured gates passed" is a well-defined ordering regardless of what those gates mean
statistically.

## Thresholds need calibration, not intuition

Every numeric threshold in `filter` (`--min-policy-margin-cp`, `--max-score-swing-cp`,
`--min-depth-reached`, ...) is a value someone picked, not a value derived from measuring what it
actually does to your data. The same threshold can mean something completely different across
datasets, engines, networks, and search depths. Two commands exist specifically to replace
guessing with measurement:

- **`calibrate`** sweeps a threshold across values you supply and reports coverage
  (kept/dropped/`coverage_pct`) and drop-reason counts per value, on your actual dataset, using the
  exact same `evaluate_quality`/`QualityConfig` gates `filter` uses. Run it before picking a
  threshold, not after.
- **`audit`** compares each engine's shallow ("student") observations against its own much deeper
  ("teacher") observation, reporting bestmove-mismatch rate and score error in cp per student
  depth. This is the closest thing shogiesa has to "ground truth" — not a universal one, only
  "what a much deeper search from the same engine would have said" — and it's the right tool for
  answering "is my chosen `--depths` value actually good enough," which no static threshold can
  tell you on its own.
- **`tune`** merges the two: it grid-sweeps `filter` thresholds like `calibrate` does, but for
  each combined threshold configuration also reports `audit`'s teacher/student mismatch rate
  *restricted to the records that configuration would keep*. Coverage alone can't tell you
  whether a gate is trustworthy — a config that keeps 90% of a dataset but whose kept records
  disagree with a deeper teacher 15% of the time is a worse config than one that keeps 60% at a
  2% mismatch rate, even though the first number ("coverage") looks better in isolation. `tune`
  never picks a single "correct" threshold for you: `--report` computes the Pareto frontier over
  (coverage, mismatch-rate) and hands back three candidates — broad (most data), strict (most
  trustworthy), balanced (the best trade-off) — because whether a training run wants quantity or
  reliability is a decision about *your* pipeline, not something shogiesa can infer from the data.

If you take away one thing from this document: shogiesa's numbers describe *what the engine
output*, faithfully and without any statistical interpretation layered on top. Deciding what those
numbers *mean* for your specific training pipeline is what `calibrate`/`audit`/`tune` are for.
