# shogiesa

**将棋の餌。** Shogi training-data feed for NNUE engines.

shogiesa は将棋エンジンに食わせる高品質な教師局面を作るためのデータ生成ツールです。

## What it is

- CSA / KIF 棋譜から局面（SFEN）を抽出する
- USI エンジンで局面にラベル（評価値・最善手）を付ける
- 不安定局面をフィルタして訓練データを出力する

## What it is NOT

- 将棋エンジンではありません
- NNUE トレーナーではありません
- GUI ではありません

## Installation

```bash
git clone https://github.com/kent-tokyo/shogiesa
cd shogiesa
cargo build --release
# binary: target/release/shogiesa
```

## Quick start

```bash
# 1. Extract positions from CSA game records
shogiesa extract --input ./games --out positions.jsonl

# 2. Label positions with engine evaluations
shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --engine-name myengine \
  --depths 4,6,8 \
  --out observations.jsonl

# 3. Check dataset quality
shogiesa report   --input observations.jsonl
shogiesa validate --input observations.jsonl
```

## Commands

### `extract` — position extraction

```bash
shogiesa extract \
  --input ./games \          # file or directory of .csa files
  --out positions.jsonl
  --min-ply 20               # skip openings (default: 1)
  --max-ply 180
  --every-n-plies 2          # sample every 2 plies
  --dedup                    # deduplicate by SFEN
```

KIF `変化` (variation/branch) blocks are extracted too, each as its own set of positions with a
`source.path` suffixed `#varN@ply` (e.g. `game.kif#var1@2`) so they never collide with the
mainline's positions or with each other — `split --by-source` puts them in separate files.
Variations always branch from the mainline; a variation nested inside another variation isn't
supported.

### `label` — engine evaluation

```bash
shogiesa label \
  --input positions.jsonl \
  --engine ./engine-binary \
  --engine-name myengine \   # optional; falls back to USI id name
  --depths 4,6,8 \           # search depths
  --timeout-ms 10000 \
  --multipv 2 \              # optional; populates observations[].policy_margin_cp
  --out observations.jsonl
```

By default, appends observations to existing records — safe to run multiple times with
different depths, but re-running the same depth adds a duplicate. `--multipv N` (N≥2) sends
`setoption name MultiPV value N` and records how far the bestmove beats the runner-up
(`policy_margin_cp`) — a low margin means the label is a weak teaching signal even when a
bestmove exists. Every rank the engine reports is kept in `observations[].candidates` (each with
its own `multipv`/`bestmove`/`score`/`score_bound`/`pv`), not just the top two used for
`policy_margin_cp` — empty unless MultiPV≥2 was actually used, so ordinary single-PV labeling
gains no extra output. `score_bound` (`exact`/`lowerbound`/`upperbound`) marks whether a
candidate's score is a confirmed evaluation or a search bound — a bound-tagged runner-up is
never trusted for `policy_margin_cp`.

`label` runs on `--jobs` parallel engine processes via a rayon thread pool scoped to that one
label invocation.

`--skip-existing` skips a requested depth if this engine already has an observation reaching at
least that depth — useful for cheaply resuming a large labeling run. `--replace-existing`
overwrites an existing observation at the same depth instead of duplicating it, for
intentionally re-labeling. Both are mutually exclusive, and both key off the depth the engine
*actually reached*, not the one requested — an engine that stops early (e.g. a forced mate) can
report a shallower depth than asked for, and these flags account for that rather than silently
duplicating or failing to skip.

### `stability` — compute stability scores

```bash
shogiesa stability --input observations.jsonl --out observations.jsonl
```

Adds `stability.score_swing_cp` (max − min cp across observations) and `stability.bestmove_agreement`
to each record. If the record was labeled by 2+ distinct engines (see `label --engine-name`),
also adds `stability.engine_bestmove_agreement` and `stability.engine_score_swing_cp` — computed
from each engine's *deepest* observation, so a depth mismatch between engines can itself surface
as disagreement (intentional: it's each engine's best-available answer). `None` with fewer than
2 engines represented.

### `filter` — stability-based filtering

```bash
shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --exclude-mate \
  --require-bestmove-agreement \
  --require-engine-agreement \
  --out train.jsonl
```

Keeps only positions passing the given stability/eval-range/phase criteria. See `shogiesa filter --help` for the full flag list.
`--require-engine-agreement` / `--max-engine-score-swing-cp` mirror
`--require-bestmove-agreement` / `--max-score-swing-cp` but compare across distinct *engines*
(a teacher-ensemble disagreement signal) instead of across depths of one engine — both are a
no-op on positions labeled by only one engine.

### `mine` — hard-position mining

```bash
shogiesa mine --input observations.jsonl --blunder-threshold 200 --out hard.jsonl
```

Extracts positions around large eval swings (blunders) and/or a `--losing-threshold`.

### `balance` — rebalance dataset distribution

```bash
shogiesa balance --input positions.jsonl --by phase --by side --out balanced.jsonl
```

Buckets by `phase`/`side`/`eval-bucket` and takes an equal number from each bucket.

### `split` / `sample` — dataset slicing

```bash
shogiesa split  --input positions.jsonl --by-source --out-dir by_game/
shogiesa split \
  --input positions.jsonl \
  --train train.jsonl --valid valid.jsonl --test test.jsonl \
  --valid-frac 0.1 --test-frac 0.1 --seed 42
shogiesa sample --input positions.jsonl --count 10000 --seed 1 --out sample.jsonl
```

`split --by-source` writes one file per source game plus a `manifest.json` (input path, schema
version, per-file counts). `split --train/--valid/--test` does a seeded ratio split instead —
every position from the same source game is assigned to exactly one of the three splits (no
same-game leakage across train/valid/test), and it writes a `manifest.json` with the seed,
requested fractions, and the *actual* per-split position/source counts (these naturally deviate
from the requested fractions since games vary in length). `sample` deterministically selects N
positions.

### `pack` / `unpack` — binary format

```bash
shogiesa pack   --input observations.jsonl --out data.shgpk
shogiesa unpack --input data.shgpk --out observations.jsonl
```

Compact binary encoding of the JSONL schema for faster loading by trainers.

### `report` — dataset statistics

```bash
shogiesa report --input observations.jsonl
```

Outputs: position count, ply range, phase/side distribution, duplicate SFENs, tag mismatches,
source dominance, balance warnings, and — once positions are labeled — cp/mate ratio, average
score swing (plus a histogram), average policy margin, eval-bucket × phase / eval-bucket ×
side cross-tabs, and (for positions labeled by 2+ distinct engines) an engine-disagreement rate.

### `validate` — data integrity

```bash
shogiesa validate --input observations.jsonl          # warnings only, exit 0
shogiesa validate --input observations.jsonl --strict  # exit 1 on any issue (CI)
```

Checks: broken JSON, invalid SFENs, duplicate SFENs, `side_to_move` tag vs SFEN mismatch.

## JSONL Schema

```json
{
  "schema_version": 4,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": {
    "kind": "csa",
    "path": "games/example.csa",
    "ply": 24
  },
  "tags": {
    "phase": "middlegame",
    "side_to_move": "black",
    "in_check": false,
    "has_capture": false
  },
  "observations": [
    {
      "engine": "myengine",
      "engine_version": "0.1.0",
      "depth": 8,
      "score": { "kind": "cp", "value": 43 },
      "bestmove": "7g7f",
      "nodes": 123456,
      "time_ms": 120,
      "pv": ["7g7f", "8h7g"],
      "policy_margin_cp": 310,
      "candidates": [
        { "multipv": 1, "bestmove": "7g7f", "score": { "kind": "cp", "value": 43 }, "score_bound": "exact", "pv": ["7g7f", "8h7g"] },
        { "multipv": 2, "bestmove": "2g2f", "score": { "kind": "cp", "value": -267 }, "score_bound": "exact", "pv": ["2g2f"] }
      ]
    }
  ]
}
```

Score is either `{"kind":"cp","value":N}` or `{"kind":"mate","moves":N}`. `policy_margin_cp` and
`candidates` are only present when `label --multipv 2` (or higher) was used.

## Pipeline

```bash
shogiesa extract --input ./games --out positions.jsonl

shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --depths 4,6,8 \
  --out observations.jsonl

shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --out train.jsonl

your-trainer --scored train.jsonl
```

shogiesa connects to engines via SFEN, JSONL, and USI — no engine-internal dependencies.

## Limitations

| Item | Status |
|---|---|
| KIF `変化` (variation/branch) moves | extracted as separate positions (`source.path` suffixed `#varN@ply`), but only relative to the mainline — a variation nested inside another variation is not supported |
| `Sfen`/`Board` legality checking | syntactic only, no full legal-move generation (by design) |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
