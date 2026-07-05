# shogiesa

**将棋の餌。** Shogi training-data feed for NNUE engines.

shogiesa は将棋エンジンに食わせる高品質な教師局面を作るためのデータ生成ツールです。

## What it is

- CSA / KIF 棋譜から局面（SFEN）を抽出する
- USI エンジンで局面にラベル（評価値・最善手）を付ける
- 不安定局面をフィルタして訓練データを出力する

## What it is NOT

- 将棋エンジンではありません
- NNUE トレーナーではありません
- GUI ではありません

## Installation

```bash
git clone https://github.com/kent-tokyo/shogiesa
cd shogiesa
cargo build --release
# binary: target/release/shogiesa
```

## Quick start

```bash
# 1. Extract positions from CSA game records
shogiesa extract --input ./games --out positions.jsonl

# 2. Label positions with engine evaluations
shogiesa label \
  --input positions.jsonl \
  --engine ./your-engine \
  --engine-name myengine \
  --depths 4,6,8 \
  --out observations.jsonl

# 3. Check dataset quality
shogiesa report   --input observations.jsonl
shogiesa validate --input observations.jsonl
```

## Commands

### `extract` — position extraction

```bash
shogiesa extract \
  --input ./games \          # file or directory of .csa files
  --out positions.jsonl
  --min-ply 20               # skip openings (default: 1)
  --max-ply 180
  --every-n-plies 2          # sample every 2 plies
  --dedup                    # deduplicate by SFEN
```

KIF `変化` (variation/branch) blocks are extracted too, each as its own set of positions with a
`source.path` suffixed `#varN@ply` (e.g. `game.kif#var1@2`) so they never collide with the
mainline's positions or with each other — `split --by-source` puts them in separate files.
Variations always branch from the mainline; a variation nested inside another variation isn't
supported. Each such record also carries `source.root_id` (shared with its mainline),
`source.variation_id` (e.g. `"var1"`), and `source.branch_from_ply` — see "JSONL Schema" below;
`split --train/--valid/--test` uses `root_id` (falling back to the `path` suffix when absent) to
keep a mainline and its variations from leaking across train/valid/test.

### `label` — engine evaluation

```bash
shogiesa label \
  --input positions.jsonl \
  --engine ./engine-binary \
  --engine-name myengine \   # optional; falls back to USI id name
  --depths 4,6,8 \           # search depths
  --timeout-ms 10000 \
  --multipv 2 \              # optional; populates observations[].policy_margin_cp
  --out observations.jsonl
```

By default, appends observations to existing records — safe to run multiple times with
different depths, but re-running the same depth adds a duplicate. `--multipv N` (N≥2) sends
`setoption name MultiPV value N` and records how far the bestmove beats the runner-up
(`policy_margin_cp`) — a low margin means the label is a weak teaching signal even when a
bestmove exists. Every rank the engine reports is kept in `observations[].candidates` (each with
its own `multipv`/`bestmove`/`score`/`score_bound`/`pv`), not just the top two used for
`policy_margin_cp` — empty unless MultiPV≥2 was actually used, so ordinary single-PV labeling
gains no extra output. `score_bound` (`exact`/`lowerbound`/`upperbound`) marks whether a
candidate's score is a confirmed evaluation or a search bound — a bound-tagged runner-up is
never trusted for `policy_margin_cp`.

`label` streams its input line-by-line through a bounded reader / worker-pool / writer pipeline
instead of loading the whole dataset into memory — memory use scales with `--jobs`, not with
dataset size. Each of the `--jobs` workers owns one long-lived engine process, launched once and
reused across every position it processes (not respawned per position). Output preserves input
order by default (a bounded reorder buffer holds back an out-of-order result until its
predecessors have been written); `--unordered-output` writes results as they arrive instead,
trading order for throughput when input order doesn't matter downstream.

`--skip-existing` skips a requested depth if this engine already has an observation reaching at
least that depth — useful for cheaply resuming a large labeling run. `--replace-existing`
overwrites an existing observation at the same depth instead of duplicating it, for
intentionally re-labeling. Both are mutually exclusive, and both key off the depth the engine
*actually reached*, not the one requested — an engine that stops early (e.g. a forced mate) can
report a shallower depth than asked for, and these flags account for that rather than silently
duplicating or failing to skip. Every observation also records `requested_depth` — the depth that
was actually asked for on that call — so `--replace-existing` only treats two observations as the
same slot when both the achieved depth *and* `requested_depth` match (a legacy observation with no
recorded `requested_depth` still matches on achieved depth alone, for older JSONL).

`--manifest PATH` writes a run manifest (engine/depths/MultiPV config, launch failures, coverage
stats) — see "Run manifests" further down.

`--cache-dir PATH` caches each observation as a small JSON file, sharded into subdirectories by
the first two hex characters of a content hash over `(sfen, engine name, engine version, engine
options, engine binary fingerprint, requested depth, multipv, schema version)` — no database,
just files you can inspect or delete by hand. Cache writes are atomic (temp file + rename), so a
crash mid-write can never leave a torn file visible to a concurrent reader — relevant since a
cache dir is meant to be shared across simultaneous `label` runs. Labeling (running the engine)
is the dominant cost of the whole pipeline, so repeated experiments over the same positions
(tuning a downstream filter config, resuming after a crash, sharing a labeling budget across
datasets) reuse a cached observation instead of re-running the engine. Cache hit/miss counts
appear in `--manifest`. The engine must still be launchable even on a run that hits the cache on
every position — the cache saves search time, not engine availability (the probe launch and each
worker's engine start happen regardless of hit rate).

`--engine-fingerprint-mode content|metadata|none` (default `content`) controls whether the engine
binary itself also contributes to the cache key, on top of its USI-reported `id name`/`id
version` — those strings are controlled by the engine and aren't guaranteed to change after a
local rebuild, so relying on them alone risks a cache hit silently reusing labels produced by a
different executable. `content` hashes the binary's bytes (read once, negligible next to actually
running search); `metadata` hashes its canonical path/size/mtime instead (cheaper, but
invalidates on every rebuild into a fresh path even when the bytes are identical — e.g. a CI job
that builds into a new directory each run); `none` restores the original behavior of trusting the
USI id strings alone. If `--engine` names a bare command resolved via `PATH` (which reading/
stat-ing the binary can't follow the way process spawning does), `content`/`metadata` fall back
to `none`'s behavior for that run with a warning, rather than failing `label` outright.

### `stability` — compute stability scores

```bash
shogiesa stability --input observations.jsonl --out observations.jsonl
```

Adds `stability.score_swing_cp` (max − min cp across observations) and `stability.bestmove_agreement`
to each record. If the record was labeled by 2+ distinct engines (see `label --engine-name`),
also adds `stability.engine_bestmove_agreement` and `stability.engine_score_swing_cp` — computed
from each engine's *deepest* observation, so a depth mismatch between engines can itself surface
as disagreement (intentional: it's each engine's best-available answer). `None` with fewer than
2 engines represented. Both agreement checks exclude special bestmove tokens (`resign`/`win`/`none`,
see `bestmove_kind` under "JSONL Schema") from the comparison — one engine giving up isn't an
opinion about which move is best, so it's neither counted as agreement nor disagreement.

### `filter` — stability-based filtering

```bash
shogiesa filter \
  --input observations.jsonl \
  --max-score-swing-cp 150 \
  --exclude-mate \
  --require-bestmove-agreement \
  --require-engine-agreement \
  --out train.jsonl
```

Keeps only positions passing the given stability/eval-range/phase criteria. See `shogiesa filter --help` for the full flag list.
`--eval-min`/`--eval-max` compare against Black-perspective cp (positive = good for Black,
regardless of whose turn it was), not the raw side-to-move-relative value USI reports — see
`Observation.score_perspective` under "JSONL Schema".
`--require-engine-agreement` / `--max-engine-score-swing-cp` mirror
`--require-bestmove-agreement` / `--max-score-swing-cp` but compare across distinct *engines*
(a teacher-ensemble disagreement signal) instead of across depths of one engine — both are a
no-op on positions labeled by only one engine.

`--require-exact-score` excludes positions where any observation's score is a search bound
(lowerbound/upperbound) rather than a confirmed evaluation. `--require-policy-margin` excludes
positions where no observation has a computed `policy_margin_cp` at all — unlike
`--min-policy-margin-cp` (a no-op when every margin is unset, since it only checks margins that
were actually computed), this requires a margin to exist in the first place.

`--min-depth-reached N` excludes positions where any *non-mate* observation's achieved `depth` is
below `N`. Mate observations are exempt: an engine stopping short of the requested depth is
dominantly caused by finding a forced mate (a confirmed, high-confidence result), not a weak
search — gating on depth without this exemption would penalize the most reliable observations.

`--require-requested-depth-reached` excludes positions where any *non-mate* observation's achieved
`depth` fell short of its own `requested_depth` (the depth `label` asked for, recorded per
observation — see `Observation.requested_depth` below). Unlike `--min-depth-reached` (a fixed
floor you pick), this checks each observation against the depth it was itself asked to reach —
useful once different observations in the same dataset were requested to different depths. A
no-op on observations with no recorded `requested_depth` (labeled before this field existed).
Mate is exempt for the same reason as `--min-depth-reached`.

`--manifest PATH` (also on `balance`/`sample`/`pack`/`label`, below) writes a run manifest — see
"Run manifests" further down.

`--dry-run` reports what would be kept/dropped (and why, via the same drop-reason breakdown) as
a normal run, without writing `--out` — `--out` isn't required in this mode. Combine with
`--manifest` to get a structured preview of a filter config's effect with no output file.

`--explain-out PATH` writes every rejected record to a JSONL file, each line
`{"record": ..., "quality": ...}` pairing the dropped record with its full `QualityDecision`
(every failing reason, not just the first one used for the stderr breakdown) — useful for
routing rejected positions to manual review or a future re-labeling pass. Works standalone or
combined with `--dry-run`/`--manifest`.

### `mine` — hard-position mining

```bash
shogiesa mine --input observations.jsonl --blunder-threshold 200 --out hard.jsonl
```

Extracts positions around large eval swings (blunders) and/or a `--losing-threshold`.

### `balance` — rebalance dataset distribution

```bash
shogiesa balance --input positions.jsonl --by phase --by side --out balanced.jsonl
```

Buckets by `phase`/`side`/`eval-bucket` and takes an equal number from each bucket. `eval-bucket`
buckets on Black-perspective cp, so the same absolute outcome (e.g. "Black is winning by 300")
lands in the same bucket regardless of whose turn the position was. Reads its input twice (once to
tally each bucket's size, since `--target` defaults to the smallest bucket's size; once to rank)
and keeps a bounded top-`--target` heap per bucket instead of materializing the whole dataset, so
memory scales with `(bucket count × target)`, not with dataset size.

### `select` — re-labeling candidates

```bash
shogiesa select \
  --input observations.jsonl \
  --strategy uncertain \
  --count 100000 \
  --seed 42 \
  --out relabel_candidates.jsonl
```

`filter` decides what's good enough to train on; `select` picks what's worth a second, deeper
label pass — re-labeling an entire dataset at higher depth costs the same whether 1% or 100% of
it is actually weak, so `select` spends that budget on the positions most likely to need it.
`--strategy`:

- `uncertain` — weak or missing label signals: non-exact score, no computed `policy_margin_cp`,
  `requested_depth` not reached, or engine disagreement. Ranks by `evaluate_quality`'s own
  pass-fraction (the same gate logic `filter` uses, via `require-exact-score`/
  `require-policy-margin`/`require-requested-depth-reached`/`require-engine-agreement` all
  enabled at once) — worst first. `--min-policy-margin-cp N` optionally also weighs in a
  too-small (rather than merely absent) margin, mirroring `filter`'s flag of the same name.
- `hard` — large eval swings, bestmove disagreement, and blunder-adjacency (reusing `mine`'s
  blunder-window detection via `--blunder-threshold`/`--blunder-window`) — worst first.
- `coverage` — positions from the thinnest phase/side/eval-bucket combinations (reusing
  `balance`'s bucket key) — thinnest first.

Unlike `sample`/`balance`, output is in ranked order (most-worth-a-look first), not restored to
input order — a re-labeling queue is more useful read top-to-bottom by priority. Ties within a
rank break deterministically by `--seed`, the same mechanism `sample` uses.

`--strategy uncertain`/`coverage` stream the input and keep a bounded top-`--count` heap instead
of materializing the whole dataset, so memory scales with `--count`, not with dataset size
(`coverage` reads its input twice — once to tally bucket sizes, once to rank — since a bucket's
size can't be known until every position naming it has been seen). `--strategy hard` still
materializes the full dataset: its blunder-adjacency signal fundamentally needs a whole game's
positions grouped together, which isn't safe to stream without assuming the input is contiguously
grouped by source.

### `split` / `sample` — dataset slicing

```bash
shogiesa split  --input positions.jsonl --by-source --out-dir by_game/
shogiesa split \
  --input positions.jsonl \
  --train train.jsonl --valid valid.jsonl --test test.jsonl \
  --valid-frac 0.1 --test-frac 0.1 --seed 42
shogiesa sample --input positions.jsonl --count 10000 --seed 1 --out sample.jsonl
```

`split --by-source` writes one file per source game plus a `manifest.json` (input path, schema
version, per-file counts). Keeps at most `--max-open-writers` (default 256) output files open at
once — a corpus with more distinct source games than that reuses the least-recently-written file
handle, closing (and, if that source is seen again, reopening in append mode) whichever source
wrote longest ago, so FD usage stays bounded regardless of source-game count.
`split --train/--valid/--test` does a seeded ratio split instead —
every position from the same source game is assigned to exactly one of the three splits (no
same-game leakage across train/valid/test — this includes a KIF `変化` variation's positions,
which are assigned alongside their mainline rather than independently, since they share a parent
position), grouped by `source.root_id` when present (falling back to stripping the `path`'s
`#varN@ply` suffix for JSONL/extractors that never set `root_id`, e.g. CSA), and it writes a
`manifest.json` with the seed, requested fractions, and the *actual* per-split position/source
counts (these naturally deviate from the requested fractions since games vary in length).
`sample` deterministically selects N positions, streaming the input and keeping a bounded
top-`--count` heap (by `seeded_hash`) instead of materializing the whole dataset, the same
technique `select --strategy uncertain/coverage` uses.

### `pack` / `unpack` — binary format

```bash
shogiesa pack   --input observations.jsonl --out data.shgpk
shogiesa unpack --input data.shgpk --out observations.jsonl
```

Compact binary encoding of the JSONL schema for faster loading by trainers.

### Run manifests

`filter`/`balance`/`sample`/`pack`/`label` accept `--manifest PATH` to write a JSON provenance
record alongside their normal output: shogiesa version, git sha (embedded at build time),
schema/pack format version, the full command line, the input file's path and a content hash
(`input_hash`, with `fingerprint_algorithm` naming the algorithm — `blake3`, chosen because its
digest for a given input is stable across Rust toolchain versions, unlike the
`std::collections::hash_map::DefaultHasher` used before; this is a "did the input change between
runs" marker, not a verifiable integrity checksum), records read/kept/dropped, drop-reason
counts, labeled/unlabeled record counts, MultiPV candidate coverage, `score_bound` distribution,
requested-depth total/underreach counts, and (for `filter`) the resolved quality configuration or
(for `label`) the engine name/depths/MultiPV/engine options/job count, engine-launch-failure
count, and (when `--cache-dir` is used) cache hit/miss counts and `engine_fingerprint_mode`. It's
opt-in and additive — no effect on the command's normal output when omitted. `split` doesn't have
`--manifest`: it already writes its own tailored `manifest.json` (see above).

### `report` — dataset statistics

```bash
shogiesa report --input observations.jsonl
```

Outputs: position count, ply range, phase/side distribution, duplicate SFENs, tag mismatches,
source dominance, balance warnings, and — once positions are labeled — cp/mate ratio, an
observation-level `score_bound` (exact/lowerbound/upperbound) distribution (unconditional — this
reflects `Observation.score_bound`, so it's meaningful even without MultiPV), average score swing
(plus a histogram), average policy margin, an eval-bucket histogram plus eval-bucket × phase /
eval-bucket × side cross-tabs (bucketed on Black-perspective cp, so the histogram/cross-tabs share
one reference frame regardless of whose turn each position was), (for positions labeled by 2+
distinct engines) an engine-disagreement rate, a special-bestmove rate (fraction of labeled
positions with at least one `resign`/`win`/`none` observation — excluded from both disagreement
rates above, not counted as either agreement or disagreement), (when `label --multipv N` (N≥2)
was used) MultiPV-candidate coverage and a separate `score_bound` distribution scoped to those
candidates, and (when any observation has a recorded `requested_depth`) a requested-depth
underreach rate. Streams its input in a single pass and never materializes the record set; memory
scales with distinct SFEN/source-file count, not total records.

### `validate` — data integrity

```bash
shogiesa validate --input observations.jsonl          # warnings only, exit 0
shogiesa validate --input observations.jsonl --strict  # exit 1 on any issue (CI)
```

Checks: broken JSON, invalid SFENs, duplicate SFENs, `side_to_move` tag vs SFEN mismatch.

## JSONL Schema

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

Score is either `{"kind":"cp","value":N}` or `{"kind":"mate","moves":N}`. `score_perspective`
(`side_to_move`/`black`) says which side a `cp` value's sign is relative to — USI's `info score
cp` is side-to-move-relative by protocol convention and `label` never converts it, so this is
always `side_to_move` on data `label` produces; it defaults to `side_to_move` on older JSONL that
predates this field, which is exactly what that data always meant. `score_bound`
(`exact`/`lowerbound`/`upperbound`) marks whether the bestmove's own score is a confirmed
evaluation or a search bound, independent of MultiPV — it defaults to `exact` on older JSONL that
predates this field. `requested_depth` is the depth `label` asked the engine to search to
(`depth` is what it actually reached — they can differ, e.g. a forced mate found short of the
request); it's absent/`null` on JSONL labeled before this field existed. `policy_margin_cp` and
`candidates` are only present when `label --multipv 2` (or higher) was used. `bestmove_kind`
(absent for an ordinary move) is `"resign"`/`"win"`/`"no_move"` when the engine's `bestmove` line
is one of those literal USI tokens rather than an ordinary move string, so consumers can tell "the
engine considers the position decided" apart from "the engine picked a normal move" without
string-matching `bestmove` themselves.

`source` also carries optional `root_id`/`variation_id`/`branch_from_ply` fields, e.g. for a KIF
`変化` branch:

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

`root_id` is shared by the mainline and every variation branching from it (the mainline's own
`path`); `variation_id`/`branch_from_ply` are `null` on the mainline itself. All three are absent
on CSA-extracted positions (no variation concept) and on JSONL predating this field.

## Pipeline

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

shogiesa connects to engines via SFEN, JSONL, and USI — no engine-internal dependencies.

## Limitations

| Item | Status |
|---|---|
| KIF `変化` (variation/branch) moves | extracted as separate positions (`source.path` suffixed `#varN@ply`), but only relative to the mainline — a variation nested inside another variation is not supported |
| `Sfen`/`Board` legality checking | syntactic only, no full legal-move generation (by design) |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
