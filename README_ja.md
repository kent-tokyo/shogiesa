# shogiesa

**将棋の餌。** NNUE エンジン向け将棋訓練データフィード。

shogiesa は将棋エンジンに食わせる高品質な教師局面を作るためのデータ生成ツールです。

## これは何か

- CSA 棋譜から局面（SFEN）を抽出する
- USI エンジンで局面にラベル（評価値・最善手）を付ける *(開発予定)*
- 不安定局面をフィルタして訓練データを出力する *(開発予定)*

## これは何でないか

- 将棋エンジンではありません
- NNUE トレーナーではありません
- GUI ではありません

shogiesa は「良い訓練局面を作る道具」に徹します。探索・評価・学習は別のツールの仕事です。

## インストール

Rust ツールチェーンが必要です（[rustup](https://rustup.rs) 推奨）。

```bash
git clone https://github.com/kent-tokyo/shogiesa
cd shogiesa
cargo build --release
# バイナリ: target/release/shogiesa
```

## クイックスタート

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

## コマンドリファレンス

### extract — 局面抽出

```bash
shogiesa extract \
  --input ./games \      # ファイルまたはディレクトリ（.csa）
  --out positions.jsonl  # 出力 JSONL

# よく使うオプション
  --min-ply 20           # 序盤を除く（デフォルト: 1）
  --max-ply 180          # 終局間際を除く
  --every-n-plies 2      # 2手に1局面をサンプリング
  --dedup                # 同一 SFEN を除去
```

### report — 統計レポート

```bash
shogiesa report --input positions.jsonl
```

局面数・ply分布・phase分布・手番分布・重複SFEN数・タグ不一致数などを出力します。

### validate — データ整合性チェック

```bash
shogiesa validate --input positions.jsonl
```

壊れた行・重複 SFEN・`side_to_move` タグと SFEN 手番の不一致を検出します。
問題がある場合は exit 1 を返します。

## JSONL スキーマ

各局面は 1 行の JSON として出力されます。

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
      "score_cp": 43,
      "bestmove": "7g7f",
      "nodes": 123456
    }
  ]
}
```

`observations` には `shogiesa label` で評価値・最善手が追記されます（開発予定）。

## パイプライン全体像

```bash
# Step 1: 局面抽出
shogiesa extract --input ./games --out positions.jsonl

# Step 2: エンジンでラベリング（開発予定）
shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --depths 4,6,8 \
  --out observations.jsonl

# Step 3: 不安定局面を除去（開発予定）
shogiesa filter \
  --input observations.jsonl \
  --min-stability 0.85 \
  --out train.jsonl

# Step 4: エンジンで学習
your-trainer --scored train.jsonl
```

shogiesa はエンジン内部に依存しません。SFEN・JSONL・USI という安定したフォーマットで接続します。

## 現在の制限事項

| 項目 | 状態 |
|---|---|
| `in_check` / `has_capture` タグ | 常に `false`（着手生成が必要） |
| KIF 形式 | 未対応（将来 `shogiesa-kif` として追加予定） |
| `label` コマンド | 未リリース（`shogiesa-usi` クレート開発中） |
| `filter` コマンド | 未リリース |
| バイナリパック形式 | 未実装（JSONL が安定してから追加予定） |
