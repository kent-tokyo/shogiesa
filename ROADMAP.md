# shogiesa roadmap

> 将棋の餌。Shogi training-data feed for NNUE engines.

このロードマップは、実装済みの機能と、次に検証すべき仮説を分けて管理する。
shogiesa はデータ生成・品質診断のツールであり、将棋エンジン、NNUE トレーナー、
GUI、対局大会基盤にはしない。

## 現在地

現在の基礎パイプラインは実装済みである。

### 2026-09-03 status

実装・fixture・回帰テスト・公開文書で確認できた項目には `[x]` を付けている。現在の
チェック済み範囲は、入力異常系と USI lifecycle、再ラベル判定、品質診断、root-aware split、
manifest provenance、JSONL/pack 境界（corruption fixture を含む）、recipe・API・release evidence である。未チェックの
項目は、3 OS の反復、1M/10M 規模の性能・資源測定、閾値校正、学習効果、対局効果、外部
ツールの native interoperability など、実測結果が必要なものに限定している。
`0.9.0` は GitHub `main` / `v0.9.0` tag / crates.io の公開状態を確認済みであり、次の候補
作業でも Cargo version は変更しない。

```text
CSA / KIF / match kifu
        ↓
extract / from-match
        ↓
label (USI, depth or nodes, MultiPV, cache, resume)
        ↓
stability / audit / calibrate / tune
        ↓
filter / select / mine / balance / stratify
        ↓
split / shuffle / pack
        ↓
report / distribution / validate
```

実装済みの主要な土台:

- CSA・KIF 抽出、KIF 分岐、SFEN 検証、重複排除、`in_check`/`has_capture` タグ
- USI エンジンの depth/node ラベル付け、MultiPV、bound、テレメトリ、タイムアウト、再起動、厳格モード
- 安定性・教師間不一致・品質判定、`filter`、`calibrate`、`audit`、`tune`
- hard/uncertain/coverage 選別、mine、balance、quota/group-aware `stratify`
- source/game 単位の split、決定的 shuffle、JSONL と versioned binary pack
- キャッシュの検査・検証・prune、resume、実行マニフェスト、SHA-256 provenance
- report、distribution、validate、Sekirei match kifu/opening、lineprior export の補助機能

未実施の測定は、実装済みであるかのように扱わない。特に、フィルタの閾値が
NNUE の学習結果を改善すること、処理速度・メモリ上限、Sekirei の強さへの寄与は、
別途固定条件で測定する。

## 競合に勝つための定義

ここでいう「勝つ」は棋力、Elo、エンジンの探索強度ではなく、学習データ生成と品質管理への
適合度で上回ることである。比較は次の固定軸で行い、機能の有無、診断の説明可能性、再現性、
大規模処理性能を分けて記録する。

| 評価軸 | 配点 | shogiesa が取るべき勝ち筋 |
|---|---:|---|
| データ生成パイプライン適合度 | 25 | extract → label → quality → split → export を再現可能な recipe にする |
| CSA/KIF/SFEN・局面処理 | 15 | 複数形式、分岐、異常入力、source root を保守的に処理する |
| USI教師ラベル付け | 20 | depth/node、MultiPV、bound、telemetry、timeout、cache、resume を説明可能にする |
| 品質診断・フィルタ | 15 | instability、teacher disagreement、CP/WDL conflict、drop reason を測定可能にする |
| 再現性・provenance | 10 | dataset/engine/weight/options/seed/hash を manifest に残す |
| 大規模処理性能 | 10 | 速度優位を仮定せず、RSS・中断復旧・再実行コストを実測して改善する |
| API・エコシステム | 5 | Rust API、JSONL、pack、USI、外部ツールとの境界を安定させる |

### 競合別の勝ち筋

- **YaneuraOu ScriptCollection / GenSfen**: 大量生成・既存資産・速度では競争せず、品質診断、
  filter 理由、manifest、resume、cross-engine 比較で差別化する。必要性を測った上で入出力 adapter を追加する。
- **rshogi**: Rust/NNUE/教師生成の一体感に対して、engine 内部に依存しない USI/JSONL 境界と、
  teacher weight・label provenance を強みにする。
- **cshogi**: Python と低レベル速度に対して、壊れた入力を黙って通さない検証、source-aware split、
  QualityDecision の説明可能性で勝つ。Python binding は需要が測れた場合だけ検討する。
- **rsshogi**: 汎用 Rust/Python 局面ライブラリに対して、学習用 end-to-end pipeline と品質管理を提供する。
  局面処理を重複実装せず、SFEN/JSONL を相互運用境界にする。
- **python-shogi**: 教育・小規模用途とは競合せず、その入力を取り込める tolerant ingestion と、
  規模が増えたときの streaming/USI labeling を提供する。

競合の未確認機能を推測で減点しない。速度・RSS・再利用率は同一 fixture、同一 engine 条件、
同一 hardware で測り、比較不能は未測定として残す。score は機能適合度・成熟度であり、Elo ではない。

## 優先順位

1. 壊れた入力や不安定なエンジンから、黙って誤ったデータを作らない
2. 同じ入力・設定から同じ結果を再生成できる
3. 品質ゲートを直感ではなくデータで選べる
4. 大規模データセットを中断・再開可能なコストで処理できる
5. 競合との差を、機能適合度と実測性能に分けて縮める
6. Sekirei での改善を、shogiesa 自体の機能追加と混同せず評価する

## Phase 0 — 信頼性の仕上げ（次）

### 0.1 USI テストの非フレーキー化

- `[x]` duplicate/delayed bestmove の検知を bounded wait と deterministic fixture で検証する。
- `[MEASURE]` Linux/macOS/Windows と異なる runner 負荷で反復し、flaky rate を記録する。
- `[x]` retry なしで CI が安定し、異常終了後に zombie process や成功扱い observation が残らない。
  bounded wait、strict handshake、timeout/restart fixture と clean shutdown テストで固定する。

### 0.2 ラベル再実行の同一性

- `[x]` engine、limit kind、実測/requested limit、MultiPV、options、weight hash の判定キーを固定する。
- `[MEASURE]` shallow → deep → MultiPV → weight変更を同じ入力に適用し、skip/replace/cache 件数を照合する。
- `[x]` 古い observation の誤温存と意図しない重複がなく、判定キーが manifest/docs と一致する。
  depth/node、実測 limit、MultiPV、engine/options/weight を含む判定と CLI 回帰テストで固定する。

### 0.3 入力形式と manifest の境界

- `[x]` `split` 独自 manifest の理由と nested KIF `変化` の非対応を明記する。
- `[x]` schema v1–v11/pack の互換性表を作る。
- `[x]` malformed CSA/KIF、CP932、variation、終端なし、壊れた JSONL を fixture 化する。
- `[x]` contract check が異常系 fixture の必須存在と代表 marker を検査し、fixture の差し替えや
 空ファイル化を検出する。
- `[x]` CLI の fixture-backed extract test が malformed/unterminated input の有効 prefix と
  KIF variation provenance を end-to-end で検証する。
- `[x]` CLI の `validate` が共有 broken JSONL fixture を通常モードでは警告付き成功、strict
  モードでは非ゼロ終了として扱うことを固定する。
- `[x]` 通常モードは診断付き skip、`validate --strict` は非ゼロ終了、pack round-trip の期待値が固定される。
  malformed fixture、strict validation テスト、schema v1〜v11 の pack round-trip テストで固定する。

### 0.4 クロスプラットフォーム回帰

- `[MEASURE]` `cargo test`、`cargo fmt --check`、`cargo clippy --all-targets --all-features` を3 OSで実行する。
- `[GATE]` fixture 件数、drop 理由、manifest schema、出力 hash が一致し、差異は limitation として記録される。

## Phase 1 — 品質ゲートを測定可能にする

### 1.1 指標の意味を固定

- `[x]` CP、policy margin、score swing、bestmove/engine agreement、bound、mate、resign/win/none の定義を統一する。
  `docs/THEORY.md` と README の定義を実装の型・診断出力に合わせる。
- `[x]` `report`/`validate` と `docs/THEORY.md` の例を fixture で照合する。
  `report` の full stdout golden、`validate` の broken JSONL full stdout golden、clean fixture の
  normal/strict 回帰を `docs/THEORY.md` の fixture cross-check として参照する。
- `[x]` 指標を未校正の確率や強さの証拠として表現せず、filter の各理由を独立再現できる。
  指標の注意書きと理由別 filter 回帰テストを保持する。

### 1.2 CP と WDL の矛盾診断

- `[x]` teacher CP と game outcome/WDL target の符号相違を集計する `conflict-report` を実装する。
- `[x]` 終端、mainline/variation、engine/weight 別の conflict rate を比較する。
  小規模 fixture で decisive/non-decisive、mate 除外と engine/weight 別の evaluated/conflict
  件数・率を固定し、`conflict_report_includes_mainline_and_variation_records_in_same_fixture_matrix`
  で mainline / variation の両 provenance を同じ母数として確認する。
- `[x]` conflict-report の fixture-backed CLI summary を `tests/fixtures/conflict_report.golden` の
  full stdout golden として固定し、集計見出し・除外理由・engine/weight 別の率のフォーマット回帰を追加する。
- `[x]` unknown outcome を conflict と誤分類せず、対象母数と除外理由を表示する。
  `conflict_report_excludes_unknown_draw_and_mate_and_counts_cp_sign_conflicts` で固定する。

### 1.3 連続ブロック診断

- `[x]` 32局面などを集計する `block-report` を実装する。outcome、CP平均/分散、王手率、駒得/駒損、入玉、軽量な駒活性を対象にする。
- `[x]` block size、game boundary、variation boundary による統計差を比較する。
  fixture で block size 2/1 の件数差と、同じ `root_id` を持つ KIF variation の連続性を固定する。
- `[x]` block-report の block size 1/2 出力を `tests/fixtures/block_report_size1.golden` /
  `block_report_size2.golden` として外部 fixture 化し、full stdout の統計値を固定する。
- `[x]` distribution の bucket/root/WDL/result-source 出力を `tests/fixtures/distribution.golden`
  として外部 fixture 化し、full stdout の診断値を固定する。
- `[x]` source root をまたいで混ざらず、NNUE の実 feature index ではない代替指標だと明記する。
  block boundary のテストと README の proxy 明記で固定する。

### 1.4 閾値 calibration の採用

- `[x]` `tune --preset-out` で `calibrate`/`audit` 統合結果を full `QualityConfig` 付き recipe/preset として保存し、`filter --preset` へ再投入できるようにする。
- `[MEASURE]` depth/node、MultiPV、teacher 数、filter 閾値の coverage/agreement/bound率を深い teacher と比較する。
- `[GATE]` 推奨閾値に dataset/engine 固有の根拠があり、単一 score や未校正 probability に依存しない。

## Phase 2 — recipe / provenance の固定

### 2.1 dataset identity

- `[x]` `calibrate`/`audit`/`tune` の診断出力に input/output hash、schema、実行引数、件数を manifest として保存する。
- `[x]` 診断 manifest に source root、engine、weight hash の入力分布を保存し、欠落 weight は `unknown` として扱う。
- `[x]` input/output hash、schema、args、seed、source root、engine/weight provenance を manifest で追跡する。
  command-specific manifest と欠落 weight の `unknown` 表現を実装する。
- `[MEASURE]` 異なる path、入力順、worker 数で再実行し、dataset identity と order hash を比較する。
- `[GATE]` 再現に必要な情報が欠落せず、null の `opening_id` を推測で補わない。

### 2.2 split / stratify / shuffle recipe

- `[x]` source root 単位の train/valid/test split、quota、shuffle を recipe 化する。split manifest は input/output hash と source-root counts、quota は input/axis/targets、shuffle は seed/order hash を保持する。
- `[MEASURE]` split 間の source root 重複、phase/side/eval bucket、duplicate rate を確認する。
- `[x]` mainline と KIF variation が漏れず、同じ入力・seed から再生成できる。
  `split_train_valid_test_keeps_variation_with_mainline` と
  `split_train_valid_test_deterministic_with_seed` がこの境界を固定する。

### 2.3 experiment envelope の採用判断

- `[x]` draft v1 を pack/split/opening に広げる前に、shogiesa-owned `RunManifest` と cross-repo draft envelope の利用フィールド・所有境界を決める。
- `[x]` 利用可能な `veridict` checkout の schema/manifest 実装を確認し、flat 14-field 契約と shogiesa の nested draft の差分を記録した。`quietset` / `lineprior` は checkout 不在のため未測定。
- `[x]` draft schema を無条件に canonical contract とせず、採用 repo と版を明記する。
  shogiesa-local proposal は `shogiesa/schema/experiment_envelope.schema.json` の
  `envelope_version: 1` / `$id`末尾 `:1` とし、cross-repository canonical adoption は未決定と
  明記する。

## Phase 3 — 大規模実行と配布

### 3.1 ストリーミング境界

- `[x]` label/filter/report/select/balance の入力全体 materialize を増やさない。`select --strategy hard` は source-contiguous を明示した場合だけ grouped streaming を使い、既定の非連続入力は correctness 優先で materialize する。
- `[MEASURE]` 10万局面で RSS と wall time の基準値を取得し、100万局面へ拡張する。
- `[GATE]` streaming コマンドの RSS が dataset size に比例して増えない。

### 3.2 中断・再開・cache

- `[x]` partial output、resume alignment key、atomic cache write、worker restart の failure fixture を追加する。
- `[MEASURE]`途中 kill 後の再開率、再計算件数、cache 再利用率、unordered/preserve-order の差を測る。
- `[x]` durable output と manifest から安全に再開できる。
  `--resume-from` の alignment index、`resumed_count`、atomic cache write、engine restart
  の回帰テストと README の運用手順で境界を固定する。

### 3.3 throughput / resource envelope

- `[x]` 残存する性能・再現性・学習・相互運用測定を、固定条件・記録項目・完了 artifact に
  分解した measurement matrix を追加する。
- `[x]` 外部 engine を使わない fixture-backed local measurement smoke script を追加する。
  dependency cache が不足する場合は未検証として停止理由を表示し、測定ゲートを成功扱いにしない。
- `[x]` local smoke の PASS/BLOCKED 結果を日付付き validation log に保存し、release checklist
  から最新結果を参照できるようにする。
- `[MEASURE]` 100万/1000万局面で `--jobs`、search limit、cache、出力順の wall time/RSS/output size、FD数、disk headroom を記録する。
- `[GATE]` corpus、commit、engine/weight hash、options、seed、hardware を結果に添付し、目標値を運用手順に明記する。

### 3.4 JSONL / pack 配布境界

- `[x]` JSONL を canonical な検査・差分形式として維持し、pack は magic/version/endian/compatibility test/unpack 経路を維持する。
- `[x]` pack を直接編集する一次形式と誤解させず、JSONLへ戻して検査できる。README と
  `docs/design/schema_compatibility.md` に境界を明記し、schema v1〜v11 と current pack の
  round-trip fixture/test を保持する。
- `[x]` pack fixture の round-trip/manifest test を local measurement smoke に組み込み、
  interop evidence から参照する。
- `[x]` malformed JSONL、pack bad magic、truncated header の corruption fixture と CLI failure/
  manifest-count test を追加する。
- `[x]` unsupported future pack version の corruption fixture と明示拒否テストを追加し、
  current format 11 以外を成功扱いしない境界を固定する。
- `[x]` pack の trailing bytes を corruption として拒否する fixture/test を追加し、空 EOF と
  truncated record を区別する。
- `[x]` pack version の little-endian 境界を wrong-endian fixture/test で固定し、外部依存なしの
  header 解釈回帰を追加する。
- `[x]` pack header 後の record-level truncation fixture/test を追加し、途中レコードを EOF として
  成功扱いしない境界を固定する。
- `[x]` pack corruption fixture ごとの CLI エラー文言（magic、version、header/record truncation）を
  回帰チェックし、失敗理由の可観測性を固定する。
- `[x]` shogiesa-pack の library 単体テストでエラー種別・文言と clean EOF を固定し、batch
  `decode` が途中レコードを成功扱いしない strict boundary を実装する。
- `[x]` pack API のエラー分類（magic、header/version、record truncation、clean EOF）を公開 API
  boundary / schema compatibility docs に整理し、単体テストへの導線を追加する。

## Phase 4 — Sekirei での効果検証

これは shogiesa の機能完成度ではなく、生成データが downstream の学習に有効かを測る段階である。

### 4.1 固定データセットと学習条件

- `[x]` 固定 train/valid/test split、dataset manifest、baseline/filtered/mined/balanced recipe、同じ teacher/weight/学習予算の手順を保存するための recipe template を追加する。
- `[x]` 各 recipe が input hash、split seed、label config、filter config から再生成できる。
  `docs/design/dataset_recipe_template.md` の固定コマンド、arm 別 artifact、manifest hash
  要件で再生成の入力を固定する。

### 4.2 学習指標

- `[x]` validation loss/WDL、データ量、ラベル計算コスト、再現性を recipe ごとに記録する測定 protocol を追加する。
- `[MEASURE]` validation loss/WDL、データ量、ラベル計算コスト、再現性を recipe ごとに比較し、shuffle の seed 数も増やして再確認する。
- `[GATE]` 改善が固定 split と複数 seed で再現し、data quality と training/search の改善を分離して報告する。

### 4.3 実戦への転写

- `[MEASURE]` 必要な recipe だけ固定 opening suite と反復対局で評価する。対局数、seed、SPRT/信頼区間、比較対象を記録する。
- `[GATE]` 一回の対局結果を Elo や一般的な強さの証拠にせず、dataset recipe の効果として報告する。

## Phase 5 — 競合適合度の再評価と公開

### 5.1 相互運用 fixture

- `[x]` 既存の CSA/KIF/SFEN/JSONL/pack/USI fixture を相互運用 evidence table に整理し、外部ツール固有の互換性は未測定として分離する。
- `[MEASURE]` import/export の欠落、合法性、source provenance、処理時間をツールごとに比較する。
- `[x]` 「対応している」と言える形式は round-trip または明示的な loss report がある。
  `docs/interop_evidence.md` で shogiesa-side round-trip と外部互換性未測定を分離する。

### 5.2 7軸スコアの更新

- `[x]` 配点（25/15/20/15/10/10/5）ごとの implementation-fit evidence table を `docs/` に保存する。性能未測定分は点数に混ぜない。
- `[MEASURE]` 機能適合度と、同一条件で測った wall time/RSS/cache 再利用率を別表にする。
- `[x]` shogiesa の点数は現在実装に対する評価として更新し、未測定の速度・学習効果を点数に混ぜない。
  `docs/competitor_evidence.md` の 76/100 は implementation-fit のみとし、性能を 0 点にする。

### 5.3 API・エコシステムの仕上げ

- `[x]` Rust API の最小利用例、JSONL schema、pack/unpack、USI boundary を versioned docs にする。
- `[MEASURE]` clean checkout から quick start、主要 CLI help、fixture pipeline を再実行する。
- `[x]` 外部利用者が engine 内部依存なしに extract → label → filter → export を再現できる。
  Rust API、JSONL、pack/unpack、USI の境界と quick-start を `docs/api_boundary.md` と README
  に分離して記載する。

### 5.4 リリース判断

- `[x]` release checklist に test、format、clippy、manifest、fixture、docs、互換性を追加する。
- `[x]` 競合比較表、制限事項、未測定項目、代表 dataset の再現手順を公開前に確認する。
  `docs/competitor_evidence.md`、`docs/release_checklist.md`、recipe template、validation log
  を相互参照できる状態にする。
- `[x]` 「学習データ品質管理に強い」という主張は証拠付きで行い、「最速」「最強」「最高 Elo」とは主張しない。
  implementation-fit evidence、制限事項、未測定項目を release checklist と docs に分離する。

## 保留・非目標

以下は現時点で優先しない。

- nested KIF variations の完全対応（必要性と入力実態の測定待ち）
- shogiesa 内への NNUE 学習、対局 tournament、GUI、分散学習サービスの実装
- `quietset`、`lineprior`、`veridict` との draft envelope の無条件な共通化
- absolute Elo や competitor ranking として解釈できない、未完了または小規模な測定結果の一般化

## 完了条件

各フェーズは、コードが存在するだけでなく、該当する `[BUILD]`・`[MEASURE]`・`[GATE]`
の証拠が揃った時点で完了とする。重い測定が未実施の場合は「未検証」と記録し、
実装済みという理由だけで性能・品質・強さを保証しない。

軽量な文書・fixture・schema の整合性は `bash scripts/check_repository_contract.sh` で確認し、
依存取得を伴う test/clippy と大規模測定は `bash scripts/release_readiness.sh` および
個別の測定記録で別管理する。
