# shogiesa

**将棋の餌。** NNUE エンジン向け将棋訓練データフィード。

shogiesa は将棋エンジンに食わせる高品質な教師局面を作るためのデータ生成ツールです。

## これは何か

- CSA / KIF 棋譜から局面（SFEN）を抽出する
- USI エンジンで局面にラベル（評価値・最善手）を付ける
- 不安定局面をフィルタして訓練データを出力する

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
shogiesa extract --input ./games --out positions.jsonl

# 2. エンジンで評価ラベルを付ける
shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --engine-name myengine \
  --depths 4,6,8 \
  --out observations.jsonl

# 3. データ品質を確認
shogiesa report   --input observations.jsonl
shogiesa validate --input observations.jsonl
```

## コマンドリファレンス

### extract — 局面抽出

```bash
shogiesa extract \
  --input ./games \          # ファイルまたはディレクトリ（.csa）
  --out positions.jsonl
  --min-ply 20               # 序盤を除く（デフォルト: 1）
  --max-ply 180              # 終局間際を除く
  --every-n-plies 2          # 2手に1局面をサンプリング
  --dedup                    # 同一 SFEN を除去
```

### label — エンジンでラベル付け

```bash
shogiesa label \
  --input positions.jsonl \
  --engine ./engine-binary \
  --engine-name myengine \   # 省略可; USI の id name にフォールバック
  --depths 4,6,8 \           # 探索深さ（カンマ区切り）
  --timeout-ms 10000 \       # 深さごとのタイムアウト（ミリ秒）
  --out observations.jsonl
```

既存 observations がある場合は追記します。異なる深さで複数回実行しても安全です。

### stability — 安定度スコアの算出

```bash
shogiesa stability --input observations.jsonl --out observations.jsonl
```

`stability.score_swing_cp`（observations間のcp最大-最小差）と`stability.bestmove_agreement`を各レコードに付加します。

### filter — 安定度に基づくフィルタリング

```bash
shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --exclude-mate \
  --require-bestmove-agreement \
  --out train.jsonl
```

安定度・eval範囲・phase等の条件を満たす局面のみ残します。全フラグは`shogiesa filter --help`を参照してください。

### mine — 難局面のマイニング

```bash
shogiesa mine --input observations.jsonl --blunder-threshold 200 --out hard.jsonl
```

evalの大きな揺れ（blunder）周辺の局面、および`--losing-threshold`で劣勢局面を抽出します。

### balance — データセット分布の均等化

```bash
shogiesa balance --input positions.jsonl --by phase --by side --out balanced.jsonl
```

`phase`/`side`/`eval-bucket`でバケット分けし、各バケットから同数を採用します。

### split / sample — データセットの分割・抽出

```bash
shogiesa split  --input positions.jsonl --by-source --out-dir by_game/
shogiesa sample --input positions.jsonl --count 10000 --seed 1 --out sample.jsonl
```

`split`はsourceゲームごとに1ファイル出力、`sample`はN局面を決定的に選択します。

### pack / unpack — バイナリ形式

```bash
shogiesa pack   --input observations.jsonl --out data.shgpk
shogiesa unpack --input data.shgpk --out observations.jsonl
```

JSONLスキーマをコンパクトなバイナリ形式にエンコードし、トレーナー側の読み込みを高速化します。

### report — 統計レポート

```bash
shogiesa report --input observations.jsonl
```

局面数・ply分布・phase分布・手番分布・重複SFEN数・タグ不一致数・
source dominance・balance warnings などを出力します。

### validate — データ整合性チェック

```bash
shogiesa validate --input observations.jsonl           # 警告のみ表示、exit 0
shogiesa validate --input observations.jsonl --strict  # 問題あれば exit 1（CI 用）
```

壊れた JSON 行・不正 SFEN・重複 SFEN・`side_to_move` タグと SFEN 手番の不一致を検出します。

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
      "score": { "kind": "cp", "value": 43 },
      "bestmove": "7g7f",
      "nodes": 123456,
      "time_ms": 120,
      "pv": ["7g7f", "8h7g"]
    }
  ]
}
```

スコアは `{"kind":"cp","value":N}` または `{"kind":"mate","moves":N}` の形式です。

## パイプライン全体像

```bash
# Step 1: 局面抽出
shogiesa extract --input ./games --out positions.jsonl

# Step 2: エンジンでラベリング
shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --depths 4,6,8 \
  --out observations.jsonl

# Step 3: 不安定局面を除去
shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --out train.jsonl

# Step 4: エンジンで学習
your-trainer --scored train.jsonl
```

shogiesa はエンジン内部に依存しません。SFEN・JSONL・USI という安定したフォーマットで接続します。

## 現在の制限事項

| 項目 | 状態 |
|---|---|
| KIF の `変化`（分岐）手順 | 未抽出 — 本譜のみパースする |
| `Sfen`/`Board` の合法性検証 | 構文レベルのみ。完全な合法手生成はしない（意図的な設計） |

## ライセンス

[MIT](LICENSE-MIT) または [Apache-2.0](LICENSE-APACHE) のデュアルライセンスです。
