# shogiesa

**将棋の餌。** Shogi training-data feed for NNUE engines.

shogiesa は [Sekirei](https://github.com/kent-tokyo/sekirei) に食わせる高品質な教師局面を作るためのデータ生成ツールです。

## What it is

- CSA 棋譜から局面（SFEN）を抽出する
- USI エンジンで局面にラベル（評価値・最善手）を付ける *(coming soon)*
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
# バイナリ: target/release/shogiesa
```

## Quick start

```bash
# 1. CSA 棋譜から局面を抽出
shogiesa extract \
  --input ./games \
  --out positions.jsonl

# 2. データセットの統計を確認
shogiesa report --input positions.jsonl

# 3. データ整合性チェック
shogiesa validate --input positions.jsonl
```

`label` と `filter` は次フェーズで追加予定です。

## JSONL スキーマ

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
  "observations": []
}
```

`observations` には `shogiesa label` で評価値・最善手が追記されます。

## Sekirei との関係

```bash
shogiesa extract --input ./games --out positions.jsonl
shogiesa label   --input positions.jsonl --engine ./sekirei --depths 4,6,8 --out observations.jsonl
shogiesa filter  --input observations.jsonl --min-stability 0.85 --out train.jsonl

cargo run --release -p sekirei-train -- --scored train.jsonl
```

shogiesa は Sekirei の内部に依存しません。SFEN・JSONL・USI という安定したフォーマットで接続します。

## Limitations

- `in_check` / `has_capture` タグは現在常に `false`（着手生成が必要）
- KIF 形式は未対応（`shogiesa-kif` として将来追加予定）
- `label` / `filter` コマンドは未リリース
