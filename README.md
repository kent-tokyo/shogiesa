# shogiesa

**将棋の餌。** Shogi training-data feed for NNUE engines.

shogiesa は将棋エンジンに食わせる高品質な教師局面を作るためのデータ生成ツールです。

## What it is

- CSA 棋譜から局面（SFEN）を抽出する
- USI エンジンで局面にラベル（評価値・最善手）を付ける
- 不安定局面をフィルタして訓練データを出力する *(coming soon)*

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

`filter` is coming in the next phase.

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
  --out observations.jsonl
```

Appends observations to existing records — safe to run multiple times with different depths.

### `report` — dataset statistics

```bash
shogiesa report --input observations.jsonl
```

Outputs: position count, ply range, phase/side distribution, duplicate SFENs, tag mismatches, source dominance, balance warnings.

### `validate` — data integrity

```bash
shogiesa validate --input observations.jsonl          # warnings only, exit 0
shogiesa validate --input observations.jsonl --strict  # exit 1 on any issue (CI)
```

Checks: broken JSON, invalid SFENs, duplicate SFENs, `side_to_move` tag vs SFEN mismatch.

## JSONL Schema

```json
{
  "schema_version": 1,
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
      "pv": ["7g7f", "8h7g"]
    }
  ]
}
```

Score is either `{"kind":"cp","value":N}` or `{"kind":"mate","moves":N}`.

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
  --min-stability 0.85 \
  --out train.jsonl            # coming soon

your-trainer --scored train.jsonl
```

shogiesa connects to engines via SFEN, JSONL, and USI — no engine-internal dependencies.

## Limitations

| Item | Status |
|---|---|
| `in_check` / `has_capture` tags | always `false` (requires move generation) |
| KIF format | not supported (planned as `shogiesa-kif`) |
| `filter` command | not yet released |
| binary pack format | not yet implemented |
| parallel labeling (`--jobs`) | not yet implemented |
