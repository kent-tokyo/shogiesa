# shogiesa

> 将棋の餌。NNUE・USI利用者向けの再現可能な学習データ生成ツール。

shogiesaはCSA、KIF、KI2、match-runner棋譜を点検可能な学習データへ変換します。
SFEN局面の抽出、USI教師によるラベル付け、不安定な局面の除外、JSONLまたはbinary packへの
書き出しを担当します。特定のエンジンやtrainerに依存しない独立したproductです。

workspaceのreleaseは`0.11.8`です。
[GitHub Release](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.8)、
[CHANGELOG](CHANGELOG.md)、
[v0.11.8検証記録](docs/release_validation_2026-10-24_v0.11.8.md)を参照してください。検証記録では
release候補と公開後の証拠を分けています。

## 対象範囲

shogiesaが担当するのは、棋譜の取り込み、局面の抽出と保守的な検証、重複排除、外部USIに
よるラベル付け、品質指標、選別、manifest、JSONL/pack出力、診断です。公開境界には
CSA/KIF/KI2、SFEN、USI、version付きJSONLとpackを使います。

将棋エンジン、探索実装、NNUE trainer、GUI、対局管理、cloud trainingは対象外です。
特定のエンジン、trainer、weightを必須依存にせず、診断値だけで学習効果やEloを主張しません。

## インストール

```bash
git clone https://github.com/kent-tokyo/shogiesa.git
cd shogiesa
cargo build --release
```

CLIは`target/release/shogiesa`に生成されます。

## クイックスタート

```bash
shogiesa extract --input ./games --recursive --out positions.jsonl \
  --min-ply 20 --every-n-plies 2

shogiesa label --input positions.jsonl --engine ./your-usi-engine \
  --depths 4,6,8 --out labeled.jsonl

shogiesa report --input labeled.jsonl
shogiesa filter --input labeled.jsonl --max-score-swing-cp 150 --out train.jsonl
```

ディレクトリ抽出は既定では直下だけを読みます。`--recursive`はsymlinkを追跡せず、
`.csa`、`.kif`、`.ki2`を相対パス順で処理します。全オプションの正本は
`shogiesa <command> --help`です。

## 処理の流れ

```text
CSA / KIF / KI2 / match kifu
        ↓
extract / from-match → label → stability / audit / calibrate / tune
        ↓
filter / select / mine / balance / stratify → split / shuffle → pack
        ↓
report / distribution / validate / dataset-diff
```

再現するrunでは、hash、コマンド、seed、engine/weight identity、option、manifestを保存します。
取得できないidentityは推測せず`unknown`とします。record、pack、manifest、reportのversionは
独立しているため、layoutを複製せず`shogiesa-core`の型付きparserを使ってください。

## コマンド

| 区分 | コマンド | 用途 |
|---|---|---|
| 取り込み | `extract`, `from-match` | 棋譜をJSONL局面へ変換する。 |
| ラベル | `label`, `cache`, `merge-observations` | USI教師とobservationを管理する。 |
| 品質 | `stability`, `filter`, `calibrate`, `audit`, `tune` | 品質signalを付与・点検・較正する。 |
| 選別 | `select`, `mine`, `balance`, `stratify`, `sample` | 難局面や不足bucketを選ぶ。 |
| 再現性 | `split`, `shuffle`, `recipe`, `dataset-diff` | root、順序、成果物identityを管理する。 |
| 診断 | `report`, `quality-report`, `distribution`, `validate`, `conflict-report`, `block-report` | 統計と整合性問題を報告する。 |
| 交換 | `pack`, `unpack`, `lineprior export`, `make-gate-openings` | 外部ツール向けに変換する。 |

`recipe`が実行するのは型付きのshogiesa stageだけです。任意のshell commandは実行しません。

## データ契約

JSONLがcanonical formatです。各recordはschema version、指し手後のSFEN、source、tags、
任意のobservation、stability、resultを持ちます。

```json
{
  "schema_version": 11,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": { "kind": "csa", "path": "games/example.csa", "ply": 24 },
  "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": false },
  "observations": []
}
```

Rust consumerは`shogiesa-core`へ一方向に依存し、
`shogiesa_core::schema::parse_json_line`で読み込みます。旧schemaの既定値を適用し、
未対応versionは明示的に拒否します。
大規模datasetは`PositionRecordJsonlReader<R: BufRead>`で全件を保持せず、物理行番号付きで
逐次処理できます。

binary packはversion付きの転送形式です。点検やdiffではJSONLへ戻してください。
詳細は[schema/pack互換性](docs/design/schema_compatibility.md)にあります。

自動qualificationでは、JSONLとpackから同じversion付きsemantic summaryを生成できます。

```bash
shogiesa quality-report --input train.jsonl --out quality.json
shogiesa quality-report --input train.shgpk --out packed-quality.json
```

このreportはcorpus identity、coverage、parse failureを記録します。学習品質や棋力の証明では
ありません。詳細は[quality-report互換性](docs/design/quality_report_compatibility.md)にあります。

## 文書

[文書索引](docs/README.md)から、利用ガイド、公開契約、測定手順、過去releaseの検証記録へ
移動できます。

## 証拠の境界

- SFEN検証は構文と保守的なmaterial制約を扱います。完全な合法局面判定ではありません。
- 100k/1M測定は単一macOS環境のsynthetic recordによる結果です。代表corpusや他OSの性能を
  保証しません。
- 3-seed学習比較は48局面、1 epochのpilotです。一般化性能や棋力を証明しません。
- 外部native format adapter、10M規模、match transfer、Eloは未測定です。
- provenance manifestはproducerごとに管理します。cross-tool chainは明示的なhashとadapterで
  接続し、shogiesaは共有cross-repository envelopeを公開しません。

## 開発

```bash
bash scripts/check_repository_contract.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## ライセンス

[MIT](LICENSE-MIT)または[Apache-2.0](LICENSE-APACHE)を選べます。
