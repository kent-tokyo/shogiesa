# shogiesa

> 将棋の餌。NNUE エンジンのための将棋学習データ生成ツール。

shogiesa は棋譜から検査可能で再現できる学習データを作ります。SFEN局面の抽出、USI教師による
ラベル付け、品質・provenanceの記録、外部トレーナー（Sekireiなど）向けデータセットの準備を行います。

現在のソース版は `0.9.2` です。`v0.9.2` のtagとGitHub commitは公開済みですが、crates.io公開は
認証エラーで未完了です。ローカル検証の正本は
[`docs/release_validation_2026-09-04.md`](docs/release_validation_2026-09-04.md)です。

## 役割

shogiesa はデータ生成・品質診断のツールです。将棋エンジン、NNUEトレーナー、GUI、対局管理、
クラウドサービスではありません。JSONL、SFEN、USI指し手、manifest、任意のbinary packを出力しますが、
データセットが棋力を改善したという主張はしません。

主な機能:

- CSA/KIF と match-runner棋譜の取り込み
- 保守的なSFEN検証、重複排除、source root provenance、診断
- timeout・restart・cache・resumeを備えたUSI教師ラベル付け
- stability、quality、conflict、block、distribution、整合性の診断
- 決定的なsplit、quota sampling、shuffle、recipe、JSONL/packの運用

## インストール

```bash
git clone https://github.com/kent-tokyo/shogiesa.git
cd shogiesa
cargo build --release
# バイナリ: target/release/shogiesa
```

## クイックスタート

```bash
# 1. CSA/KIF棋譜から、指し手後の局面を抽出する
shogiesa extract --input ./games --out positions.jsonl --min-ply 20 --every-n-plies 2

# 2. USIエンジンでラベル付けする
shogiesa label --input positions.jsonl --engine ./sekirei --depths 4,6,8 --out labeled.jsonl

# 3. 診断し、根拠を確認してからフィルタする
shogiesa report --input labeled.jsonl
shogiesa filter --input labeled.jsonl --min-stability 0.85 --out train.jsonl
```

全オプションの正本は実行バイナリです。

```bash
shogiesa --help
shogiesa label --help
shogiesa recipe run --help
```

## 処理の流れ

KIFの`変化`は本譜と同じ`root_id`を共有する別系列として抽出されます。親より深い字下げの入れ子分岐は
親局面から再生し、`#var1@2#var2@3`・`variation_id: "var1.var2"`のように完全な系列を残します。
同じ深さか浅い`変化`は本譜からのsiblingです。分岐の結果は対局結果ではないため`unknown`になります。

```text
CSA / KIF / match kifu
        ↓
extract / from-match → label → stability / audit / calibrate / tune
        ↓
filter / select / mine / balance / stratify → split / shuffle → pack
        ↓
report / distribution / validate
```

再現したい結果では、入力、engine binary/options、weight、seed、実行引数を固定してください。
manifestを出すコマンドは取得できるidentityとhashを記録します。値が無い場合は推測せず`unknown`のままです。

## コマンド一覧

| 区分 | コマンド | 用途 |
|---|---|---|
| 取り込み | `extract`, `from-match` | CSA/KIFまたはmatch-runner棋譜をJSONL局面へ変換する。 |
| ラベル | `label`, `cache`, `merge-observations` | USI教師の実行、cache管理、複数passの統合。 |
| 品質 | `stability`, `filter`, `calibrate`, `audit`, `tune` | 不安定性・品質signalを付与、点検、較正する。 |
| 選別 | `select`, `mine`, `balance`, `stratify`, `sample` | 難局面・不足bucketの選別とサンプリング。 |
| 再現性 | `split`, `shuffle`, `recipe plan/run/verify`, `dataset-diff` | source root保護、順序固定、recipe実行、成果物比較。 |
| 診断 | `report`, `distribution`, `validate`, `conflict-report`, `block-report` | 統計、missing bucket、整合性、proxy診断を出す。 |
| 交換 | `pack`, `unpack`, `lineprior export`, `make-gate-openings` | 外部ツール向け変換・入力作成。 |

`recipe`が受け付けるのは型付きのshogiesa stageだけで、任意のshell commandは実行しません。
`recipe run`はstage出力をstaging経由で確定し、identityが一致する成功済み成果物だけを再利用します。
実験時に残す記録は
[`docs/design/dataset_recipe_template.md`](docs/design/dataset_recipe_template.md)を参照してください。

## データ契約

JSONLはstream処理・diff・点検に適したcanonical formatです。各局面はschema version、指し手後の
SFEN、source、tags、任意のobservation/stability/resultを持ちます。

```json
{
  "schema_version": 11,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": { "kind": "csa", "path": "games/example.csa", "ply": 24 },
  "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": false, "has_capture": true },
  "observations": []
}
```

binary packはmagic header、version、endian定義、unpack経路を持つ転送形式です。primary formatとして
byte列を編集せず、JSONLへ戻して検査・diffしてください。互換性とエラー分類は
[`docs/design/schema_compatibility.md`](docs/design/schema_compatibility.md)にあります。

## 診断値の読み方

`score.cp`、policy margin、stability、agreement、`QualityDecision.score`は診断値です。確率、
各手の正しさ、エンジン強さの証拠ではありません。閾値は固定corpusとteacher設定で較正します。
詳しくは[`docs/THEORY.md`](docs/THEORY.md)を参照してください。

KIFの`変化`は本譜と同じ`root_id`を共有する別source pathとして抽出します。このリリースで対応するのは
本譜起点の平坦な`変化`だけで、入れ子variationの方言には対応していません。変化手順は実対局ではないため、
outcomeは`unknown`です。

## 文書の案内

| 知りたいこと | 文書 |
|---|---|
| 現在地と測定gate | [`ROADMAP.md`](ROADMAP.md) |
| schema/packの互換性 | [`docs/design/schema_compatibility.md`](docs/design/schema_compatibility.md) |
| Rust API境界 | [`docs/api_boundary.md`](docs/api_boundary.md) |
| 指標と品質signalの限界 | [`docs/THEORY.md`](docs/THEORY.md) |
| 相互運用の根拠と未測定範囲 | [`docs/interop_evidence.md`](docs/interop_evidence.md) |
| 学習効果・gate評価手順 | [`docs/design/training_effect_measurement.md`](docs/design/training_effect_measurement.md)、[`docs/SEKIREI_GATE_EVALUATION.md`](docs/SEKIREI_GATE_EVALUATION.md) |
| lineprior実験 | [`docs/LINEPRIOR_DOGFOOD.md`](docs/LINEPRIOR_DOGFOOD.md) |
| リリース証跡と確認項目 | [`docs/release_validation_2026-09-04.md`](docs/release_validation_2026-09-04.md)、[`docs/release_checklist.md`](docs/release_checklist.md) |
| feature-fit比較 | [`docs/competitor_evidence.md`](docs/competitor_evidence.md) |

## 制限と証拠の境界

- SFENは構文と保守的なmaterial制約を検査します。完全な合法手生成ではありません。
- GenSfen、rshogi、cshogi、rsshogi、python-shogiとのnative相互運用は未測定です。
- throughput、RSS、学習効果、対局結果、Eloは、corpus・commit・hardware・engine/weight・budgetを
  記録した日付付き結果がない限り未測定です。
- cross-repository experiment envelopeはshogiesaが管理するdraftで、共有標準ではありません。

## 開発

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash scripts/check_repository_contract.sh
```

contract checkは軽量です。`scripts/release_readiness.sh`はcargo検査も実行し、依存取得やnetworkの
失敗を成功扱いせずに報告します。

## ライセンス

[MIT](LICENSE-MIT) または [Apache-2.0](LICENSE-APACHE) のデュアルライセンスです。
