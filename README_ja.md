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

shogiesa の品質指標(`score.cp`、`policy_margin_cp`、`score_swing_cp`、`bestmove_agreement`、
`QualityDecision.score`)が実際に何を意味するか — どれも較正された確率ではありません —
そして `calibrate`/`audit` がどのように経験則を実測に置き換えるかについては、
[`docs/THEORY.md`](docs/THEORY.md)(英語)を参照してください。

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

### `extract` — 局面抽出

```bash
shogiesa extract \
  --input ./games \          # ファイルまたはディレクトリ（.csa/.kif）
  --out positions.jsonl
  --min-ply 20               # 序盤を除く（デフォルト: 1）
  --max-ply 180              # 終局間際を除く
  --every-n-plies 2          # 2手に1局面をサンプリング
  --dedup                    # 同一 SFEN を除去
```

KIF の `変化`（分岐）ブロックも抽出されます。それぞれ独立した局面群として扱われ、
`source.path` に `#varN@ply` サフィックス（例: `game.kif#var1@2`）が付くため、本譜の局面や
他の変化と衝突しません — `split --by-source` では別ファイルに分かれます。変化は常に本譜から
分岐する前提で、変化の中にさらに変化がある入れ子構造には対応していません。こうしたレコードは
`source.root_id`（本譜と共有）、`source.variation_id`（例: `"var1"`）、
`source.branch_from_ply` も持ちます — 詳細は後述の「JSONLスキーマ」を参照。
`split --train/--valid/--test` は `root_id`（無ければ `path` サフィックスにフォールバック）を
使い、本譜と変化がtrain/valid/testをまたがないようにします。

### `label` — エンジンでラベル付け

```bash
shogiesa label \
  --input positions.jsonl \
  --engine ./engine-binary \
  --engine-name myengine \   # 省略可; USI の id name にフォールバック
  --depths 4,6,8 \           # 探索深さ（カンマ区切り）
  --timeout-ms 10000 \
  --multipv 2 \              # 省略可; observations[].policy_margin_cp を計算
  --out observations.jsonl
```

デフォルトでは既存レコードに observation を追記します — 異なる深さで複数回実行しても安全ですが、
同じ深さを再実行すると重複が追加されます。`--multipv N`（N≥2）は `setoption name MultiPV value N`
を送り、bestmove が runner-up をどれだけ上回っているか（`policy_margin_cp`）を記録します —
margin が小さいほど、bestmove があってもラベルとしての信頼性は低いことを意味します。
エンジンが報告した全ランクは `observations[].candidates`（各要素が独自の
`multipv`/`bestmove`/`score`/`score_bound`/`pv` を持つ）に保持されます。`policy_margin_cp`
の計算に使う上位2件だけでなく全件です — MultiPV≥2 を実際に使った場合のみ値が入り、通常の
単一PVラベリングでは出力サイズが増えません。`score_bound`（`exact`/`lowerbound`/`upperbound`）
は候補のスコアが確定評価値か探索バウンドかを示し、bound タグ付きの runner-up は
`policy_margin_cp` の計算に決して使われません。

`label` は入力を1行ずつストリーミングし、bounded な reader / worker プール / writer
パイプラインで処理します — データセット全体をメモリに載せることはなく、メモリ使用量は
`--jobs` に比例します(データセットサイズには比例しません)。`--jobs` 個の各ワーカーは
エンジンプロセスを1つだけ起動し、以後すべての局面でそれを使い回します(局面ごとの
再起動はありません)。デフォルトでは出力順は入力順を保持します(到着順が入れ替わった
結果は、先行する結果が書き出されるまで bounded な reorder バッファに保持されます) —
`--unordered-output` は代わりに到着順で書き出し、順序を犠牲にしてスループットを優先します。

`--skip-existing` は、要求された深さ以上に到達済みの観測がこのエンジンから既にある場合、
その深さをスキップします — 大規模ラベリングを安価に再開するのに便利です。
`--replace-existing` は同じ深さの既存観測を重複追加ではなく上書きします（意図的な再ラベル用）。
両者は排他的で、どちらもエンジンが *実際に到達した* 深さを基準にします（要求した深さではない）
— 詰みの発見などでエンジンが早期に探索を打ち切った場合、要求より浅い深さが報告されますが、
この2つのフラグはそれを踏まえて正しく動作します（サイレントな重複追加や誤ったスキップを防ぐ）。
各観測には `requested_depth`（その呼び出しで実際に要求した深さ）も記録されます — そのため
`--replace-existing` は、到達深さと `requested_depth` の両方が一致した場合のみ同じ観測とみなします
（`requested_depth` が記録されていない旧JSONLの観測は、到達深さのみで一致とみなされます）。

`--manifest PATH` は実行マニフェスト（エンジン/深さ/MultiPV設定、起動失敗数、カバレッジ統計）
を書き出します — 詳細は後述の「実行マニフェスト」を参照。

`--cache-dir PATH` は各観測を小さなJSONファイルとしてキャッシュします。ファイルは
`(sfen, engine名, engineバージョン, engineオプション, engineバイナリのfingerprint,
requested_depth, multipv, schema_version)` に対するコンテンツハッシュの先頭2文字で
サブディレクトリに分散配置され、DBは使いません — 手で覗いたり消したりできるファイルだけです。
キャッシュへの書き込みはatomicです(一時ファイルに書いてからrename)。クラッシュで書き込み
途中のファイルが残ることはありません — cache-dirは複数の`label`実行から同時に共有される
想定なので、これは重要です。ラベル付け(エンジンの実行)はパイプライン全体で最も高コストな
処理なので、同じ局面に対する繰り返しの実験(下流のfilter設定のチューニング、クラッシュ後の
再開、複数データセット間でのラベリング予算の共有)では、エンジンを再実行する代わりに
キャッシュされた観測を再利用します。cache hit/miss件数は`--manifest` に出力されます。
全件キャッシュヒットする実行でも、エンジン自体は起動可能である必要があります(probe起動と
各workerのエンジン起動はヒット率に関係なく発生するため)。キャッシュが節約するのは探索時間で
あって、エンジンの実行環境そのものではありません。

`--engine-fingerprint-mode content|metadata|none`(デフォルト `content`)は、engineバイナリ
自体をcache keyに含めるかどうかを制御します。USIが報告する `id name`/`id version` だけに
頼らない理由は、これらの文字列はエンジン側が決めるものであり、ローカルで再ビルドしても
変わるとは限らないからです — そのため、異なる実行ファイルでラベル付けした結果を、cache hit
として黙って再利用してしまう恐れがあります。`content` はバイナリのバイト列をハッシュします
(起動時に一度だけ読み込むので、探索コストに比べれば無視できるコストです)。`metadata` は
正規化したパス・サイズ・mtimeをハッシュします(軽量ですが、バイトが同一でも新しいパスに
再ビルドするたびにキャッシュが無効化されます — 例えばCIジョブが毎回新しいディレクトリに
ビルドする場合など)。`none` はUSIのid文字列だけに頼る従来の挙動に戻します。`--engine` が
PATH経由で解決されるベア名の場合(プロセス起動と違い、読み込み/statはPATH解決を追わない
ため)、`content`/`metadata` はその実行に限り警告付きで `none` 相当の挙動にフォールバック
します — `label` 自体を失敗させることはありません。

### `cache` — `label --cache-dir` の点検・保守

```bash
shogiesa cache stats  --cache-dir .shogiesa-cache
shogiesa cache verify --cache-dir .shogiesa-cache
shogiesa cache prune  --cache-dir .shogiesa-cache --older-than-days 30
shogiesa cache prune  --cache-dir .shogiesa-cache --corrupted-only --yes
```

`cache stats` はエントリ数・合計サイズ・最古/最新エントリの経過日数、そして(各エントリに
保存された `Observation.engine` フィールドから読み取った)エンジン別分布を表示します。
`cache verify` は壊れた(パースできない)エントリを検出します。**スコープに関する注記**:
cache key(`(sfen, engine名/バージョン, engineオプション, engineバイナリのfingerprint,
requested_depth, multipv, schema_version)`)は一方向ハッシュです — キャッシュされたJSON
ペイロードは素の `Observation` であり、schema_version/fingerprint のメタデータは保存
されていません。そのため `verify` は「このエントリは古いschemaでキャッシュされた」や
「このエントリは現在のエンジンと一致しない」を検出できませんし、そう主張もしません。
これは正当性の欠陥ではありません — `SCHEMA_VERSION` とengine fingerprintは既にcache key
自体に組み込まれているため、schemaの変更やengineの変更は今後単に別のキーを生成するだけで、
古いエントリが誤って再利用されることはなく、単にディスク上の孤立したゴミになるだけです。
これが `cache prune --older-than-days N` の役割です。`cache prune` はデフォルトでdry-run
です(削除される内容を報告するだけ)— 実際に削除するには `--yes` を渡してください。
`--corrupted-only`/`--older-than-days` の少なくとも一方が必須です。両方指定した場合は
どちらかに一致するものを削除します。

### `stability` — 安定度スコアの算出

```bash
shogiesa stability --input observations.jsonl --out observations.jsonl
```

`stability.score_swing_cp`（observations間のcp最大-最小差）と `stability.bestmove_agreement`
を各レコードに付加します。2つ以上の異なるエンジンでラベル付けされている場合（`label
--engine-name` 参照）は `stability.engine_bestmove_agreement` と `stability.engine_score_swing_cp`
も追加されます — 各エンジンの *最も深い* 観測から計算されるため、エンジン間の深さの違い自体が
不一致として現れることがあります（意図的な仕様: 各エンジンの最善の回答同士を比較するため）。
エンジンが2つ未満の場合は `None` になります。どちらの一致判定も特殊bestmoveトークン
（`resign`/`win`/`none`、「JSONLスキーマ」の `bestmove_kind` 参照）を比較対象から除外します —
どちらかのエンジンが投了しただけでは「どの手が最善か」についての意見表明にはならないため、
一致とも不一致ともカウントしません。

### `filter` — 安定度に基づくフィルタリング

```bash
shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --exclude-mate \
  --require-bestmove-agreement \
  --require-engine-agreement \
  --out train.jsonl
```

指定した安定度・eval範囲・phase等の条件を満たす局面のみ残します。全フラグは
`shogiesa filter --help` を参照してください。`--eval-min`/`--eval-max` は、USIが返す
手番側視点の生の値ではなく、先手視点のcp（プラス=先手有利、手番に関わらず）と比較します —
詳細は「JSONLスキーマ」の `Observation.score_perspective` を参照。`--require-engine-agreement` /
`--max-engine-score-swing-cp` は `--require-bestmove-agreement` / `--max-score-swing-cp`
と対になりますが、1エンジン内の深さ間ではなく、異なる *エンジン* 間の不一致（teacher-ensemble
の不一致シグナル）を比較します — どちらも1エンジンのみでラベル付けされた局面では no-op です。

`--require-exact-score` は、いずれかの観測のスコアが確定評価値でなく探索バウンド
（lowerbound/upperbound）である局面を除外します。`--require-policy-margin` は、
`policy_margin_cp` が1つも計算されていない局面を除外します — `--min-policy-margin-cp`
（margin が未計算のときは no-op、つまり実際に計算されたmarginだけをチェックする）とは異なり、
そもそも margin が存在することを要求します。

`--min-depth-reached N` は、mate 以外の観測で実際に到達した `depth` が `N` 未満の局面を
除外します。mate の観測は除外対象外です — エンジンが要求深さより浅く止まるのは、
主に詰みを発見した場合（確定的で信頼度の高い結果）であり、探索が弱かったわけではないためです。
この除外を入れずに depth だけでゲートすると、最も信頼できる観測を誤って弾いてしまいます。

`--require-requested-depth-reached` は、mate 以外の観測で到達した `depth` が、その観測自身の
`requested_depth`（`label` が要求した深さ）に届かなかった局面を除外します。`--min-depth-reached`
（自分で決めた固定の下限値）とは異なり、各観測をそれ自身が要求された深さと比較します —
キャッシュや段階的な再ラベリングにより、同じデータセット内の観測が異なる深さを要求されている
場合に有用です。`requested_depth` が記録されていない観測(このフィールド追加前にラベル付け
されたもの)では no-op です。mate は `--min-depth-reached` と同じ理由で除外対象外です。

`--manifest PATH`（`balance`/`sample`/`pack`/`label` にもあり、後述）は実行マニフェストを
書き出します — 詳細は「実行マニフェスト」を参照。

`--dry-run` は、`--out` を書き出さずに、通常実行と同じ drop 理由内訳とともに
何が残り何が落ちるかを表示します（このモードでは `--out` は不要）。`--manifest` と組み合わせると、
出力ファイルなしでフィルタ設定の効果を構造化されたプレビューとして得られます。

`--explain-out PATH` は、落選した全レコードを JSONL ファイルに書き出します。各行は
`{"record": ..., "quality": ...}` の形式で、落選したレコードとその完全な `QualityDecision`
（stderr の内訳表示で使われる最初の理由だけでなく、失敗した全理由）を対にします —
落選局面を手動レビューや将来の再ラベル候補に回すのに便利です。`--dry-run`/`--manifest`
と組み合わせても、単独でも使えます。

### `calibrate` — 品質ゲートの閾値を較正する

```bash
shogiesa calibrate \
  --input observations.jsonl \
  --sweep-policy-margin 0,40,80,120,160 \
  --sweep-score-swing 50,100,150,200 \
  --out calibration.csv
```

`filter` の閾値(`--min-policy-margin-cp`、`--max-score-swing-cp` など)は、これまで経験則で
決めるしかありませんでした。`calibrate` は `filter` と全く同じ `shogiesa_core::evaluate_quality`/
`QualityConfig` を再利用します — CLI側に別の品質判定ロジックは増やしません — 指定した値の
範囲で閾値を掃引し、各値でどれだけの局面が残る/落ちるか、なぜ落ちるかを報告するので、
経験則ではなく自分のデータセット・エンジンに基づいて閾値を選べます。
`--sweep-policy-margin`/`--sweep-score-swing` はそれぞれ独立に掃引されます(掃引した値ごとに
CSV1行)。もう一方の次元は `--min-policy-margin-cp`/`--max-score-swing-cp` で固定値に
できます(同じフィールドを掃引する場合とは併用不可)。`filter` の他のゲートフラグ
(`--exclude-mate`、`--eval-min`/`--eval-max`、`--require-exact-score` など)もここで使え、
掃引中は全ての値で固定されます。出力は `(sweep_param, sweep_value)` ごとに1行のCSVで、
`total`/`kept`/`dropped`/`coverage_pct` と `drop_reasons` 列(最初に失敗した理由のみ、`filter`
の stderr 内訳と同じ規約)を含みます。別途、1回だけ、掃引に依存しない stderr サマリーも
表示します: `policy_margin_cp`/`score_swing_cp` の分布(50cp単位、`report` のヒストグラムと
同じ規約)、観測レベルの `score_bound` 件数、`requested_depth` 未達率、特殊bestmove率 —
掃引結果を解釈する際の文脈情報であり、閾値によって変わる値ではありません。

### `audit` — 浅いラベルと深いラベルを比較する

```bash
shogiesa audit \
  --input observations.jsonl \
  --teacher-depth 14 \
  --student-depths 6,8,10 \
  --out audit.jsonl
```

「このデータセットで、浅い深さでラベル付けすると実際にエンジンごとにどれだけのコストが
かかるか」に答えます — 既に手元にあるデータに対する純粋な分析コマンドです。1回の
`label --depths 6,8,10,14` 実行で、既に1レコードあたり深さごとに複数の同一エンジンの
`Observation` が生成されています(`Observation.depth` 参照)。各レコードについて観測を
`engine` でグループ化し(2つ以上のエンジンでラベル付けされたデータセットで、engine Aの
浅い観測とengine Bの深い観測を比較することは絶対にありません)、各エンジンの
`--teacher-depth` の観測(`requested_depth` で一致、レガシーなschema v6未満のデータでは
達成された `depth` にフォールバック)と各 `--student-depths` の観測を同じ規則で探し、
両方が存在する(engine, student_depth)の組み合わせごとに `audit.jsonl` に1行書き出します:
```json
{"sfen": "...", "source": {...}, "engine": "sekirei",
 "teacher_requested_depth": 14, "teacher_depth": 14, "teacher_score_bound": "exact",
 "teacher_underreach": false, "teacher_bestmove_kind": null,
 "student_requested_depth": 8, "student_depth": 8, "student_score_bound": "exact",
 "student_underreach": false, "student_bestmove_kind": null,
 "bestmove_match": true, "score_error_cp": -35}
```
`bestmove_match` は `bestmove_agreement` を再利用します(他の箇所と同様に resign/win/none を
比較から除外)。`score_error_cp`(どちらかが詰みの場合は `None`)は両辺を
`cp_from_black_perspective` で正規化してから差を取ります — 手番相対値の生の差ではありません。
教師観測自体が強制詰みにより `--teacher-depth` に届かなかった場合でも、そのまま教師として
使われます(`filter` の深さゲートと同じ詰み除外規約)— `teacher_underreach` は正しく
`false` になります(バグではありません)。student_depth ごとおよび全体の stderr サマリーを
表示します: 比較件数、bestmove不一致率、`|score_error_cp|` の平均/最大値、教師/生徒の
非exact率、教師/生徒の未達率、教師/生徒の特殊bestmove率。

### `tune` — 閾値のグリッドサーチとteacher depth比較を同時に行う

```bash
shogiesa tune \
  --input observations.jsonl \
  --teacher-depth 14 \
  --student-depths 6,8,10 \
  --sweep-policy-margin 0,40,80,120,160 \
  --sweep-score-swing 50,100,150,200 \
  --out tuning.csv \
  --report tuning.md
```

`calibrate` と `audit` を1つの問いに統合します: より多くのデータを残す品質ゲート設定は、
より信頼性の低いデータも残していないか? `--sweep-policy-margin` × `--sweep-score-swing`
をグリッドサーチします(各セルは組み合わせた閾値であり、`calibrate` の独立した1次元sweep
とは異なります — 1×N や N×1 のグリッドは `calibrate` と全く同じ挙動に退化するので、`tune`
は別の概念ではなく厳密な上位互換です)。各セルについて、カバレッジ(`evaluate_quality`/
`QualityConfig` 経由、`calibrate` と同じ — 別の判定ロジックはありません)と、**そのセルが
残すレコードに限定した** `audit` 形式の教師/生徒不一致メトリクスの両方を報告します。
1回のストリーミングパスで完結します: 各レコードの教師/生徒比較は(閾値に関係なく)一度だけ
計算され、そのレコードを残す全てのグリッドセルに畳み込まれます(セルごとに再計算しません)。

`--out tuning.csv` は `(policy_margin, score_swing)` セルごとに1行: カバレッジ/kept/dropped/
drop_reasons(`calibrate` と同じ規約)に加えて `audit_pairs`/不一致率/`score_error_cp` の
平均・最大/非exact率/未達率/特殊bestmove率 — audit由来の列は、そのセルにaudit pairが
ない場合は(`0.00` ではなく)空欄になります。真の0%不一致率が「データなし」と混同されない
ためです。

`--report tuning.md`(任意)は各セルの(coverage, mismatch_rate)点からPareto frontierを
計算し、3つの候補を提示します — **broad**(最大coverage)、**strict**(最小不一致率)、
**balanced**(理想的な角への最短距離 — coverage と mismatch_rate をフロンティア自身の
観測範囲に正規化してから距離を計算します。正規化しないと、coverage の範囲が
mismatch_rate よりずっと広いため、「balanced」が「broad」に潰れてしまいます)— shogiesa
が唯一の「正しい」閾値を選ぶのではありません。訓練実行ごとに量と信頼性のどちらを重視するか
は変わるため、`tune` は判定ではなくトレードオフ曲線を返します。

### `mine` — 難局面のマイニング

```bash
shogiesa mine --input observations.jsonl --blunder-threshold 200 --out hard.jsonl
```

evalの大きな揺れ（blunder）周辺の局面、および`--losing-threshold`で劣勢局面を抽出します。

### `balance` — データセット分布の均等化

```bash
shogiesa balance --input positions.jsonl --by phase --by side --out balanced.jsonl
```

`phase`/`side`/`eval-bucket`でバケット分けし、各バケットから同数を採用します。`eval-bucket`は
先手視点のcpでバケット分けするため、同じ絶対的な局面評価（例:「先手が300有利」）は手番に
関わらず同じバケットに入ります。入力を2回読みます(1回目は各バケットのサイズを集計 —
`--target` はデフォルトで最小バケットのサイズになるため、2回目で順位付けします)。
全データセットをメモリに載せる代わりにバケットごとに上位 `--target` 件だけを保持する
有界ヒープを使うため、メモリ使用量は `(バケット数 × target)` に比例し、データセットサイズには
比例しません。

### `select` — 再ラベル候補の選別

```bash
shogiesa select \
  --input observations.jsonl \
  --strategy uncertain \
  --count 100000 \
  --seed 42 \
  --out relabel_candidates.jsonl
```

`filter` は「訓練に使える局面か」を判定するコマンド、`select` は「もっと深く読み直す価値が
ある局面」を選ぶコマンドです — 全局面を高depthで再ラベルするコストは、実際に弱いのが
1%でも100%でも変わらないため、`select` はその予算を最も見込みのある局面に集中させます。
`--strategy`:

- `uncertain` — ラベルの信頼シグナルが弱い、または欠けている局面: 非確定スコア、
  `policy_margin_cp` 未計算、`requested_depth` 未達、エンジン間の不一致。`filter` と同じ
  ゲートロジック(`require-exact-score`/`require-policy-margin`/
  `require-requested-depth-reached`/`require-engine-agreement` を同時に有効化)を使う
  `evaluate_quality` 自身の通過率で順位付けします — 悪い順。`--min-policy-margin-cp N` を
  指定すると、margin が「存在しない」だけでなく「小さすぎる」局面も考慮されます
  (`filter` の同名フラグと同じ意味)。
- `hard` — evalの大きな揺れ、bestmove不一致、blunder近傍(`mine` の blunder-window 検出を
  `--blunder-threshold`/`--blunder-window` 経由で再利用)— 悪い順。
- `coverage` — phase/side/eval-bucket の組み合わせが薄いバケットから優先します
  (`balance` のバケットキーを再利用)— 薄い順。

`sample`/`balance` と異なり、出力は入力順ではなく順位順(最も見るべき局面が先頭)です —
再ラベルの待ち行列は優先順位で先頭から読む方が有用なためです。同順位内のタイブレークは
`--seed` により決定的です(`sample` と同じ仕組み)。

`--strategy uncertain`/`coverage` は全データセットをメモリに載せず、入力をストリーム処理しながら
上位 `--count` 件だけを保持する有界ヒープを使うため、メモリ使用量はデータセットサイズではなく
`--count` に比例します(`coverage` は入力を2回読みます — 1回目でバケットサイズを集計し、
2回目で順位付けします。あるバケットのサイズは、そのバケットに属する全局面を見終えるまで
確定しないためです)。`hard` 戦略は引き続き全データセットをメモリに載せます —
blunder近傍の判定は1ゲーム分の局面がまとまっている必要があり、入力がsourceごとに
連続して並んでいる前提を置かない限りストリーム処理は安全ではないためです。

### `split` / `sample` — データセットの分割・抽出

```bash
shogiesa split  --input positions.jsonl --by-source --out-dir by_game/
shogiesa split \
  --input positions.jsonl \
  --train train.jsonl --valid valid.jsonl --test test.jsonl \
  --valid-frac 0.1 --test-frac 0.1 --seed 42
shogiesa sample --input positions.jsonl --count 10000 --seed 1 --out sample.jsonl
```

`split --by-source` は source ゲームごとに1ファイル出力し、`manifest.json`(入力パス・
スキーマバージョン・ファイル別件数)も書き出します。同時に開く出力ファイルは最大
`--max-open-writers`件(デフォルト256)に制限されます — 異なるsourceゲーム数がこれを
超える場合は、最後に書き込んでから最も時間が経ったファイルハンドルを再利用します
(クローズし、そのsourceが再度出現すればappendモードで再オープンします)。これにより
sourceゲーム数によらずFD使用量が一定に保たれます。
`split --train/--valid/--test` は代わりに
シード付き比率分割を行います — 同じ source ゲームの局面は必ず3つの分割のうち1つだけに
割り当てられます（train/valid/test間の同一ゲームからのリークを防止。KIF `変化` の局面も、
親局面を共有する本譜と同じ扱いになり、独立には扱われません — `source.root_id` が
あればそれを使い、無ければ `path` の `#varN@ply` サフィックスを外したものにフォールバック
します）。この分割も
`manifest.json`（シード・要求した比率・*実際の*分割別局面/ソース件数）を書き出します —
ゲームの長さがまちまちなため実際の件数は要求した比率から自然にずれます。`sample` は
N局面を決定的に選択します。全データセットをメモリに載せる代わりに入力をストリーム処理し、
上位 `--count` 件だけを保持する有界ヒープ(`seeded_hash` によるキー)を使います —
`select --strategy uncertain/coverage` と同じ手法です。

### `pack` / `unpack` — バイナリ形式

```bash
shogiesa pack   --input observations.jsonl --out data.shgpk
shogiesa unpack --input data.shgpk --out observations.jsonl
```

JSONLスキーマをコンパクトなバイナリ形式にエンコードし、トレーナー側の読み込みを高速化します。

### 実行マニフェスト

`filter`/`balance`/`sample`/`pack`/`label` は `--manifest PATH` で JSON 形式の実行記録を
通常出力と一緒に書き出せます: shogiesa バージョン、git sha（ビルド時に埋め込み）、
スキーマ/パック形式バージョン、実行時の完全なコマンドライン、入力ファイルのパスと
コンテンツハッシュ（`input_hash`、アルゴリズム名は `fingerprint_algorithm` に記録 — 使用しているのは
`blake3` です。以前使っていた `std::collections::hash_map::DefaultHasher` と異なり、同じ入力に対する
ダイジェストがRustツールチェーンのバージョンを跨いで安定しているために選んでいます。あくまで
「前回実行から入力が変わったか」を見るためのものであり、検証可能な整合性チェックサムではありません）、
読み込み/採用/棄却件数、
棄却理由別カウント、ラベル済み/未ラベル件数、MultiPV候補カバレッジ、`score_bound` 分布、
requested_depth の合計数/未達数、そして（`filter` の場合は）解決済みの品質設定、（`label` の場合は）エンジン名/深さ/MultiPV/
エンジンオプション/ジョブ数/エンジン起動失敗数、（`--cache-dir` 使用時は）cache hit/miss件数と
`engine_fingerprint_mode` です。オプトインかつ加算的な機能であり、
省略時はコマンドの通常動作に影響しません。`split` には `--manifest` はありません
— 既に専用の `manifest.json` を書き出しているためです（前述）。

### `report` — 統計レポート

```bash
shogiesa report --input observations.jsonl
```

出力内容: 局面数・ply範囲・phase/手番分布・重複SFEN数・タグ不一致数・source dominance・
balance warnings、そしてラベル付け後は cp/mate 比率、観測レベルの `score_bound`
（exact/lowerbound/upperbound）分布（無条件表示 — `Observation.score_bound` を反映するため
MultiPV を使っていなくても意味があります）、score swing 平均（ヒストグラム付き）、
policy margin 平均、eval-bucketヒストグラムと eval-bucket × phase / eval-bucket × side の
クロス集計（いずれも先手視点のcpでバケット分けするため、手番に関わらず同じ基準で
比較できます）、（2つ以上の異なるエンジンでラベル付けされた局面については）エンジン不一致率、
特殊bestmove率（`resign`/`win`/`none` の観測を1件以上含むラベル付き局面の割合 —
上記の不一致率からは除外され、一致・不一致のどちらにもカウントされません）、
（`label --multipv N`（N≥2）を使った場合は）MultiPV候補カバレッジと、
その候補に限定した別の `score_bound` 分布、そして（`requested_depth` が記録された観測が
1件以上あれば)requested_depth の未達率を表示します。入力を1回のストリーム処理のみで走査し、
レコード集合自体はメモリに載せません — メモリ使用量は総レコード数ではなく、
異なるSFEN数・source数に比例します。

### `validate` — データ整合性チェック

```bash
shogiesa validate --input observations.jsonl           # 警告のみ表示、exit 0
shogiesa validate --input observations.jsonl --strict  # 問題あれば exit 1（CI 用）
```

壊れた JSON 行・不正 SFEN・重複 SFEN・`side_to_move` タグと SFEN 手番の不一致を検出します。

## JSONL スキーマ

各局面は 1 行の JSON として出力されます。

```json
{
  "schema_version": 8,
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
      "requested_depth": 8,
      "score": { "kind": "cp", "value": 43 },
      "score_perspective": "side_to_move",
      "score_bound": "exact",
      "bestmove": "7g7f",
      "nodes": 123456,
      "time_ms": 120,
      "pv": ["7g7f", "8h7g"],
      "policy_margin_cp": 310,
      "candidates": [
        { "multipv": 1, "bestmove": "7g7f", "score": { "kind": "cp", "value": 43 }, "score_bound": "exact", "pv": ["7g7f", "8h7g"] },
        { "multipv": 2, "bestmove": "2g2f", "score": { "kind": "cp", "value": -267 }, "score_bound": "exact", "pv": ["2g2f"] }
      ]
    }
  ]
}
```

スコアは `{"kind":"cp","value":N}` または `{"kind":"mate","moves":N}` の形式です。
`score_perspective`（`side_to_move`/`black`）は `cp` の符号がどちらの視点かを示します —
USIの `info score cp` はプロトコル上の慣習として手番側視点であり、`label` はそれを変換せず
そのまま格納するため、`label` が生成するデータでは常に `side_to_move` です。このフィールドが
ない古い JSONL では `side_to_move` がデフォルト値になります（そのデータが常に意味していた
ことそのものです）。`score_bound`（`exact`/`lowerbound`/`upperbound`）は bestmove 自身の
スコアが確定評価値か探索バウンドかを、MultiPV の有無に関わらず示します — このフィールドが
ない古い JSONL では `exact` がデフォルト値になります。`requested_depth` は `label` が
エンジンに要求した深さです（`depth` は実際に到達した深さ — 詰みを早期発見した場合などに
両者は異なります）。このフィールドが追加される前にラベル付けされた JSONL では欠落/`null`
になります。`policy_margin_cp` と `candidates` は `label --multipv 2`（以上）を使った場合のみ
存在します。`bestmove_kind`（通常の手の場合は欠落）は、エンジンの `bestmove` 行が通常の指し手
文字列ではなく `resign`/`win`/`none` のいずれかのUSIトークンだった場合に
`"resign"`/`"win"`/`"no_move"` になります — 呼び出し側が `bestmove` を自前で文字列比較
することなく、「エンジンが局面を決着とみなした」場合と「通常の手を選んだ」場合を区別できます。

`source` は任意項目として `root_id`/`variation_id`/`branch_from_ply` も持ちます。例えば
KIF `変化` の枝の場合:

```json
"source": {
  "kind": "kif",
  "path": "games/example.kif#var1@12",
  "ply": 13,
  "root_id": "games/example.kif",
  "variation_id": "var1",
  "branch_from_ply": 12
}
```

`root_id` は本譜とそこから分岐した全ての変化で共有されます（本譜自身の `path`）。
`variation_id`/`branch_from_ply` は本譜自身では `null` です。CSA抽出された局面
（変化の概念がない）や、このフィールドより前の JSONL では3つとも欠落しています。

## パイプライン全体像

```bash
shogiesa extract --input ./games --out positions.jsonl

shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --depths 4,6,8 \
  --out observations.jsonl

shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --out train.jsonl

your-trainer --scored train.jsonl
```

shogiesa はエンジン内部に依存しません。SFEN・JSONL・USI という安定したフォーマットで接続します。

## 現在の制限事項

| 項目 | 状態 |
|---|---|
| KIF の `変化`（分岐）手順 | 独立した局面群として抽出済み（`source.path` に `#varN@ply` サフィックス）だが、本譜からの分岐のみ対応 — 変化の中の変化（入れ子）は非対応 |
| `Sfen`/`Board` の合法性検証 | 構文レベルのみ。完全な合法手生成はしない（意図的な設計） |

## ライセンス

[MIT](LICENSE-MIT) または [Apache-2.0](LICENSE-APACHE) のデュアルライセンスです。
