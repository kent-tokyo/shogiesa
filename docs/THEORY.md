# What shogiesa's numbers mean

shogiesa records search output and reproducible diagnostics. It does not calibrate probabilities,
estimate Elo, or prove that a position improves training.

## Signal reference

| Signal | Definition | Do not interpret it as |
|---|---|---|
| `score.cp` | Raw USI centipawn score with an explicit `score_perspective`; `label` records `side_to_move` | win probability or a score comparable across engines, weights, and depths |
| `policy_margin_cp` | Exact MultiPV rank-1 cp minus rank-2 cp at one search limit | probability that the bestmove is correct |
| `score_swing_cp` | Maximum minus minimum cp across a record's observations | error of the deepest observation |
| `bestmove_agreement` | Agreement among ordinary moves across observations; `resign`, `win`, and `none` are excluded | proof that the agreed move is optimal |
| `engine_bestmove_agreement` | Agreement among the deepest ordinary-move observations from distinct engines | independent ground truth |
| `QualityDecision.score` | Fraction of configured quality gates that passed | a calibrated confidence score |

Mate scores and bound scores are not cp values. `policy_margin_cp` is absent when MultiPV was not
used, either leading score is mate, or rank 1/2 is a lower/upper bound.

## Score perspective

USI `info score cp` is relative to the side to move. A positive value means the current player is
favored. Commands that need a fixed viewpoint use
`cp_from_black_perspective(cp, perspective, side_to_move)` before filtering or bucketing.

Do not convert cp to a win percentage. shogiesa has no such calibration, and the same numeric cp
can mean different things for different engines, weights, limits, and positions.

## Quality score

`evaluate_quality()` computes:

```rust
let score = if configured_gates == 0 {
    1.0
} else {
    1.0 - reasons.len() as f32 / configured_gates as f32
};
```

A score of `0.75` means that three of four configured gates passed. Scores from different
`QualityConfig` values are not comparable. `select --strategy uncertain` uses this value only as
a transparent ranking key.

## Choosing thresholds

Thresholds depend on the corpus, teacher, weight, and search limit. Use the commands below rather
than treating one value as universal:

- `calibrate`: sweep one threshold and report coverage and drop reasons;
- `audit`: compare shallow observations with a deeper observation from the same engine; and
- `tune`: grid-sweep gates and report the coverage/mismatch Pareto frontier.

The deeper observation used by `audit` or `tune` is a reference for that fixed engine setup, not
universal ground truth. Broad, balanced, and strict presets express different data-volume and
agreement trade-offs; none is automatically correct.

## Test coverage and evidence limit

Fixture-backed tests verify implementation behavior: score perspective, bounds, MultiPV margin,
agreement, quality reasons, requested-depth underreach, malformed input, and stable reports.
Calibration, representative-corpus quality, and engine strength require separate measurements.
Completed resource and training-pilot runs are indexed in
[`measurements/README.md`](measurements/README.md); their limits still apply.
