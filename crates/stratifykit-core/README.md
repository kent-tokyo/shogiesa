# stratifykit-core

Domain-agnostic distribution-control primitives: bounded top-K sampling, group-aware quota
fill, and coverage classification. No knowledge of any particular record type — every function
takes a plain generic record `R` and caller-supplied closures (`bucket_key_fn`, `group_key_fn`,
`key_fn`) to map that record onto this crate's generic `FeatureKey`/`BucketKey`/`GroupKey`
(all plain `String` newtypes) concepts.

This is the boundary rule the crate exists to enforce: nothing shogi-specific — no `sfen`, no
`PositionRecord`, no phase/side/eval-bucket vocabulary — may appear in this crate's source or in
`Cargo.toml`'s dependencies. `shogiesa-stratify` is the adapter crate that supplies shogi's own
closures; `stratifykit-core` never depends on it or on any other `shogiesa-*` crate. A test
(`cargo_toml_has_no_shogiesa_dependency` in `src/lib.rs`) enforces the `Cargo.toml` half of this
mechanically, so drift here fails the build rather than needing to be caught by inspection.

## Modules

- `heap` — `HeapEntry<K, R>` and `push_bounded`: a bounded top-K `BinaryHeap` that reproduces
  full-materialize-then-`sort_by`'s exact ordering (including index-based tie-break stability) at
  O(k) memory instead of O(n).
- `hash` — `seeded_hash(seed, s)`: the deterministic tie-break/spreading hash every sampling
  function above uses to pick "which items" reproducibly given the same seed.
- `coverage` — `bucket_floor`, `mean_of`, `classify_bucket` (→ `BucketStatus`:
  `Missing`/`Under`/`Ok`/`Over`): the generic "is this bucket's observed count reasonable
  relative to the mean" classification a coverage/distribution report needs.
- `quota` — `QuotaSpec`: a hand-editable quota file shape (`by`, `quotas: BTreeMap<BucketKey,
  usize>`), self-describing so a caller reconstructs its own bucketing dimensions from the file
  itself rather than from separately-passed flags.
- `sampling` — `reservoir_sample` (bounded top-K reservoir sample) and `group_aware_fill` (quota
  fill that keeps one group — e.g. one source game — from consuming an entire bucket's quota via
  a rank + hash tie-break key, so every group's first occurrence in a bucket outranks every
  group's second occurrence, across all groups).

## Status

Not yet an independent repo or published crate. It's grown inside this workspace since being
extracted from `shogiesa-cli`'s inline quota/bucket/sampling logic, and stays here until a real
second consumer (e.g. a masstrust/quietset-style product) actually needs it outside this
workspace — extracting on spec, before that need is concrete, would just be guessing at an API
boundary nobody has stress-tested yet.
