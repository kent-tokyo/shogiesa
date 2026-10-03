# shogiesa

> 将棋の餌。NNUEエンジン向けの学習データ生成ツール。

shogiesaはCSA、KIF、KI2、match-runner棋譜を、点検可能な学習データへ変換します。SFEN局面の
抽出、USI教師によるラベル付け、不安定な局面の除外、外部トレーナー向けJSONL・binary packの
作成を担当します。

ワークスペースのバージョンは`0.10.1`です。公開workflowが完了するまでは、検証済みの最新公開版を
`0.10.0`として扱います。変更内容は[CHANGELOG](CHANGELOG.md)、検証結果は
[v0.10.1 validation log](docs/release_validation_2026-10-03.md)を参照してください。

## 役割

shogiesaの範囲はデータ生成と診断です。将棋エンジン、NNUEトレーナー、GUI、対局管理、
クラウドサービスではありません。診断値だけで学習効果やEloを主張することもありません。

主な機能:

- CSA/KIF/KI2とmatch-runner棋譜の取り込み
- SFEN検証、重複排除、variation-aware provenance、診断
- MultiPV、timeout、restart、cache、resumeを備えたUSIラベル付け
- stability、quality、conflict、distribution、整合性のreport
- 決定的なsplit、sampling、shuffle、recipe、JSONL/pack処理

## インストール

```bash
git clone https://github.com/kent-tokyo/shogiesa.git
cd shogiesa
cargo build --release
```

CLI binaryは`target/release/shogiesa`です。

## クイックスタート

```bash
shogiesa extract --input ./games --recursive --out positions.jsonl \
  --min-ply 20 --every-n-plies 2

shogiesa label --input positions.jsonl --engine ./sekirei \
  --depths 4,6,8 --out labeled.jsonl

shogiesa report --input labeled.jsonl
shogiesa filter --input labeled.jsonl --max-score-swing-cp 150 --out train.jsonl
```

ディレクトリの抽出は既定では直下だけです。`--recursive`を付けると、配下の`.csa`、`.kif`、
`.ki2`を相対パス順で読み、source pathにも相対パスを記録します。symlinkは追跡せず、入力ルート
自体がsymlinkの場合も拒否します。

全オプションの正本は実行バイナリです。

```bash
shogiesa --help
shogiesa extract --help
shogiesa label --help
shogiesa recipe run --help
```

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

再現する必要があるrunでは、入力hash、コマンド、seed、engine binary/options、weight、manifestを
残してください。取得できないidentityは推測せず`unknown`とします。

## コマンド

| 区分 | コマンド | 用途 |
|---|---|---|
| 取り込み | `extract`, `from-match` | 棋譜をJSONL局面へ変換する。 |
| ラベル | `label`, `cache`, `merge-observations` | USI教師を実行し、observationを管理する。 |
| 品質 | `stability`, `filter`, `calibrate`, `audit`, `tune` | 品質signalを付与・点検・較正する。 |
| 選別 | `select`, `mine`, `balance`, `stratify`, `sample` | 難局面や不足bucketを選ぶ。 |
| 再現性 | `split`, `shuffle`, `recipe`, `dataset-diff` | source root、順序、成果物identityを管理する。 |
| 診断 | `report`, `distribution`, `validate`, `conflict-report`, `block-report` | 統計と整合性問題を報告する。 |
| 交換 | `pack`, `unpack`, `lineprior export`, `make-gate-openings` | 外部ツール向けに変換する。 |

`recipe`が実行するのは型付きのshogiesa stageだけです。任意のshell commandは実行しません。

## データ契約

JSONLがcanonical formatです。各recordはschema version、指し手後のSFEN、source、tags、任意の
observation、stability、resultを持ちます。

```json
{
  "schema_version": 11,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": { "kind": "csa", "path": "games/example.csa", "ply": 24 },
  "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": false, "has_capture": true },
  "observations": []
}
```

binary packはversion付きの転送形式です。点検やdiffではJSONLへ戻してください。詳細は
[schema/pack compatibility](docs/design/schema_compatibility.md)にあります。

## 文書

| 知りたいこと | 文書 |
|---|---|
| 診断値の定義と限界 | [THEORY.md](docs/THEORY.md) |
| JSONL/packの互換性 | [schema_compatibility.md](docs/design/schema_compatibility.md) |
| Rust API境界 | [api_boundary.md](docs/api_boundary.md) |
| ローカルで確認した相互運用範囲 | [interop_evidence.md](docs/interop_evidence.md) |
| 再現可能なrecipe記録 | [dataset_recipe_template.md](docs/design/dataset_recipe_template.md) |
| scale・学習効果の測定 | [measurement_matrix.md](docs/design/measurement_matrix.md)、[training_effect_measurement.md](docs/design/training_effect_measurement.md) |
| 完了した測定artifact | [再現性matrix](docs/measurements/reproducibility_matrix_2026-10-03.json) |
| Sekirei・lineprior runbook | [SEKIREI_GATE_EVALUATION.md](docs/SEKIREI_GATE_EVALUATION.md)、[LINEPRIOR_DOGFOOD.md](docs/LINEPRIOR_DOGFOOD.md) |
| release確認と証跡 | [release_checklist.md](docs/release_checklist.md)、[v0.10.1 validation](docs/release_validation_2026-10-03.md) |

## 証拠の境界

- SFEN検証は構文と保守的なmaterial制約を扱います。完全な合法局面判定ではありません。
- GenSfen、rshogi、cshogi、rsshogi、python-shogiとのnative相互運用は未測定です。
- throughput、RSS、学習効果、対局結果、Eloは、corpus、commit、hardware、engine/weight、budgetを
  記録した日付付きrunがない限り未検証です。
- experiment envelopeはshogiesa管理のdraftで、共有標準ではありません。

## 開発

```bash
bash scripts/check_repository_contract.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## ライセンス

[MIT](LICENSE-MIT)または[Apache-2.0](LICENSE-APACHE)を選べます。
