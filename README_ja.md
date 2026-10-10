# shogiesa

> 将棋の餌。NNUEエンジン向けの学習データ生成ツール。

shogiesaはCSA、KIF、KI2、match-runner棋譜を点検可能な学習データへ変換します。
SFEN局面の抽出、USI教師によるラベル付け、不安定な局面の除外、JSONLまたはbinary packへの
書き出しを担当します。

workspaceと検証済みの最新公開版は`0.11.3`です。
[GitHub Release](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.3)、
[CHANGELOG](CHANGELOG.md)、
[v0.11.3検証記録](docs/release_validation_2026-10-11_v0.11.3.md)を参照してください。

## 役割

shogiesaの範囲はデータ生成と診断です。将棋エンジン、NNUEトレーナー、GUI、対局管理、
クラウドサービスではありません。診断値だけで学習効果やEloを主張することもありません。

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

shogiesa label --input positions.jsonl --engine ./sekirei \
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

再現するrunでは、入力hash、コマンド、seed、engine/weight identity、option、manifestを
保存してください。取得できないidentityは推測せず`unknown`とします。

## コマンド

| 区分 | コマンド | 用途 |
|---|---|---|
| 取り込み | `extract`, `from-match` | 棋譜をJSONL局面へ変換する。 |
| ラベル | `label`, `cache`, `merge-observations` | USI教師とobservationを管理する。 |
| 品質 | `stability`, `filter`, `calibrate`, `audit`, `tune` | 品質signalを付与・点検・較正する。 |
| 選別 | `select`, `mine`, `balance`, `stratify`, `sample` | 難局面や不足bucketを選ぶ。 |
| 再現性 | `split`, `shuffle`, `recipe`, `dataset-diff` | root、順序、成果物identityを管理する。 |
| 診断 | `report`, `distribution`, `validate`, `conflict-report`, `block-report` | 統計と整合性問題を報告する。 |
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

binary packはversion付きの転送形式です。点検やdiffではJSONLへ戻してください。
詳細は[schema/pack互換性](docs/design/schema_compatibility.md)にあります。

## 文書

| 知りたいこと | 文書 |
|---|---|
| 診断値の定義と限界 | [THEORY.md](docs/THEORY.md) |
| JSONL/packの互換性 | [schema_compatibility.md](docs/design/schema_compatibility.md) |
| Rust API境界 | [api_boundary.md](docs/api_boundary.md) |
| 相互運用の確認範囲 | [interop_evidence.md](docs/interop_evidence.md) |
| 再現可能なrecipe記録 | [dataset_recipe_template.md](docs/design/dataset_recipe_template.md) |
| 測定状況とartifact | [measurement_matrix.md](docs/design/measurement_matrix.md)、[測定索引](docs/measurements/README.md) |
| 学習比較の手順 | [training_effect_measurement.md](docs/design/training_effect_measurement.md) |
| Sekirei・lineprior runbook | [SEKIREI_GATE_EVALUATION.md](docs/SEKIREI_GATE_EVALUATION.md)、[LINEPRIOR_DOGFOOD.md](docs/LINEPRIOR_DOGFOOD.md) |
| release確認と証跡 | [release_checklist.md](docs/release_checklist.md)、[v0.11.3検証](docs/release_validation_2026-10-11_v0.11.3.md) |

## 証拠の境界

- SFEN検証は構文と保守的なmaterial制約を扱います。完全な合法局面判定ではありません。
- 100k/1M測定は単一macOS環境のsynthetic recordによる結果です。代表corpusや他OSの性能を
  保証しません。
- 3-seed学習比較は48局面、1 epochのpilotです。一般化性能や棋力を証明しません。
- native GenSfen/rshogi/cshogi/rsshogi/python-shogi adapter、10M規模、match transfer、Eloは
  未測定です。
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
