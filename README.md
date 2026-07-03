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

Appends observations to existing records — safe to run multiple times with different depths.
`--multipv 2` sends `setoption name MultiPV value 2` and records how far the bestmove beats
the runner-up (`policy_margin_cp`) — a low margin means the label is a weak teaching signal
even when a bestmove exists.

### `stability` — compute stability scores

```bash
shogiesa stability --input observations.jsonl --out observations.jsonl
```

Adds `stability.score_swing_cp` (max − min cp across observations) and `stability.bestmove_agreement` to each record.

### `filter` — stability-based filtering

```bash
shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --exclude-mate \
  --require-bestmove-agreement \
  --out train.jsonl
```

Keeps only positions passing the given stability/eval-range/phase criteria. See `shogiesa filter --help` for the full flag list.

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
score swing (plus a histogram), average policy margin, and eval-bucket × phase / eval-bucket ×
side cross-tabs.

### `validate` — data integrity

```bash
shogiesa validate --input observations.jsonl          # warnings only, exit 0
shogiesa validate --input observations.jsonl --strict  # exit 1 on any issue (CI)
```

Checks: broken JSON, invalid SFENs, duplicate SFENs, `side_to_move` tag vs SFEN mismatch.

## JSONL Schema

```json
{
  "schema_version": 2,
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
      "policy_margin_cp": 310
    }
  ]
}
```

Score is either `{"kind":"cp","value":N}` or `{"kind":"mate","moves":N}`. `policy_margin_cp` is
only present when `label --multipv 2` (or higher) was used.

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
| KIF `変化` (variation/branch) moves | not extracted — only the mainline is parsed |
| `Sfen`/`Board` legality checking | syntactic only, no full legal-move generation (by design) |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
