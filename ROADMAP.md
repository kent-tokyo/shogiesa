# shogiesa roadmap

> 将棋の餌。Shogi training-data feed for NNUE engines.

## 現在地 — 2026-09-23

`v0.9.2` のsource/tag/GitHub pushは完了している。crates.io公開は認証403で未完了であり、
公開済みとは扱わない。リリース時のローカル検証は
[`docs/release_validation_2026-09-04.md`](docs/release_validation_2026-09-04.md)を正本とする。

shogiesa は学習データの生成・診断・再現性を担う。将棋エンジン、NNUE trainer、GUI、対局基盤、
分散学習サービスは対象外である。

```text
CSA / KIF / match kifu
        ↓
extract / from-match → label → stability / audit / calibrate / tune
        ↓
filter / select / mine / balance / stratify → split / shuffle → pack
        ↓
report / distribution / validate
```

### 記号

- `[x]`: 実装・fixture/テスト・文書のいずれかでローカルに確認済み。
- `[MEASURE]`: 固定条件での外部実測が必要。未実施を成功扱いしない。
- `[GATE]`: 実測artifactが揃うまで完了しない判断条件。

## 完了済みの土台

- `[x]` CSA/KIF/match kifu抽出、保守的なSFEN検証、重複排除、source-root provenance。
- `[x]` USI depth/nodeラベル、MultiPV/bound、timeout/restart、cache/resume、strict diagnostics。
- `[x]` stability/quality、filter/calibrate/audit/tune、hard/uncertain/coverage選別。
- `[x]` root-aware split、quota/group-aware stratify、決定的shuffle、JSONL/pack、manifest/hash。
- `[x]` report/distribution/validate、conflict-report/block-report、recipe plan/run/verify。
- `[x]` malformed input、pack corruption、recipe resume、主要CLI出力のfixture/golden回帰。

## 優先順位

1. 壊れた入力・不安定な教師から、黙って誤ったデータを作らない。
2. 同一入力・設定から同じ成果物を再生成できる。
3. 品質gateを直感ではなく測定で選べる。
4. 大規模実行を中断・再開可能なコストで扱う。
5. Sekireiへの効果と、shogiesa自身の機能完成度を混同しない。

## Phase 0 — 信頼性

- `[x]` USI duplicate/delayed bestmove、timeout/restart、child cleanupの回帰を固定。
- `[x]` labelの再実行キー（engine、limit、MultiPV、options、weight）とresume/cache境界を固定。
- `[x]` malformed CSA/KIF/JSONL、CP932、flat KIF variation、終端なし、pack corruptionをfixture化。
- `[x]` SFENの過大hand/rankと`distribution`の極端な整数範囲を安全に拒否・処理する。
- `[MEASURE]` Linux/macOS/Windowsでtest/lint/fixture hashを反復し、flaky率を記録する。
- `[GATE]` OS差異はartifactとlimitationに記録し、成功の推測で埋めない。

## Phase 1 — 品質gateを測定可能にする

- `[x]` CP、policy margin、swing、agreement、bound、game resultの意味と限界を
  [`docs/THEORY.md`](docs/THEORY.md)に固定。
- `[x]` `report`/`validate`/`conflict-report`/`block-report`/`distribution`/`calibrate`の
  fixture-backed goldenを保持。
- `[x]` `tune --preset-out`と`filter --preset`で、診断したQualityConfigを再利用可能にする。
- `[MEASURE]` depth/node、MultiPV、teacher数、閾値ごとのcoverage/agreementを深いteacherと比較する。
- `[GATE]` 推奨閾値はdataset/engine固有の証拠を持ち、未校正の確率や単一scoreに依存しない。

## Phase 2 — recipe / provenance

- `[x]` input/output hash、schema、args、seed、source root、engine/weight provenanceをmanifestへ記録。
- `[x]` `dataset-diff`が順序非依存で追加・削除・変更とsource/phase/eval差分を出力。
- `[x]` split/stratify/shuffleがsource rootをまたぐleakを防ぎ、seedで再現する。
- `[x]` typed `recipe plan/run/verify`がstage identity、staging、atomic checkpoint、explicit resume、
  output hash検証を提供する。
- `[MEASURE]` path、入力順、worker数を変えた再実行でidentity/order hashを比較する。
- `[GATE]` 必要なprovenanceが欠けず、`unknown`を推測で補わない。

## Phase 3 — 大規模実行と配布

- `[x]` streaming境界、resume/cache、JSONL canonical・pack transportの役割を文書化・回帰固定。
- `[x]` `scripts/run_local_measurement_smoke.sh`とmeasurement matrixを用意。
- `[MEASURE]` 1M/10M局面でjobs、limit、cache、出力順ごとのwall time/RSS/output size/FD/diskを記録。
- `[GATE]` corpus、commit、engine/weight/options、seed、hardwareを同じresult artifactに残す。

## Phase 4 — Sekireiでの効果検証

- `[x]` fixed split、recipe arm、必要artifactを
  [`docs/design/dataset_recipe_template.md`](docs/design/dataset_recipe_template.md)に定義。
- `[x]` loss/WDL、ラベル計算費、再現性の記録形式を
  [`docs/design/training_effect_measurement.md`](docs/design/training_effect_measurement.md)に定義。
- `[MEASURE]` baseline/filtered/mined/balancedを固定teacher・budget・複数seedで比較する。
- `[GATE]` 改善は固定splitと複数seedで再現し、data qualityとtraining/search効果を分離する。

## Phase 5 — 相互運用・公開

- `[x]` CSA/KIF/SFEN/JSONL/pack/USIのローカル証拠と未測定境界を
  [`docs/interop_evidence.md`](docs/interop_evidence.md)に整理。
- `[x]` API、schema、feature-fit、release evidenceを目的別の短い文書に分離。
- `[MEASURE]` GenSfen/rshogi/cshogi/rsshogi/python-shogiとのnative import/exportを同一fixtureで測る。
- `[GATE]` 「対応」「高速」「学習効果」「Elo改善」は、それぞれ対応する再現可能な測定なしに主張しない。

## 競合に対する立ち位置

評価するのは棋力ではなく、学習データ生成・品質管理への適合度である。

| 軸 | 狙い |
|---|---|
| Pipeline | extract → label → quality → split → exportを再現可能にする。 |
| Input | 異常入力、分岐、source rootを保守的に扱う。 |
| Teacher | USI条件と観測値を説明可能に残す。 |
| Quality | instability、teacher disagreement、CP/WDL矛盾、drop理由を可視化する。 |
| Provenance | dataset/engine/weight/options/seed/hashを残す。 |
| Scale | 速度優位を仮定せず、RSS・復旧コストを測る。 |
| Ecosystem | Rust API、JSONL、pack、USIの安定した境界を保つ。 |

機能適合度の根拠は[`docs/competitor_evidence.md`](docs/competitor_evidence.md)にある。速度・RSS・
training effect・Eloは別の実測であり、同じ点数に混ぜない。

## 保留・非目標

- nested KIF variationの完全対応（入力方言と必要性の測定待ち）
- shogiesa内でのNNUE学習、対局tournament、GUI、分散学習
- quietset/lineprior/veridictとのdraft envelopeの無条件な共通化
- 単発または小規模な結果をabsolute Eloや一般的な強さに一般化すること

## 実行前の確認

```bash
bash scripts/check_repository_contract.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

軽量なcontract checkと、依存取得を伴うcargo検証・大規模測定は別管理する。
