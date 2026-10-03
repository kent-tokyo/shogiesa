# stratifykit-core

Domain-neutral primitives for bounded sampling, group-aware quota filling, deterministic hashing,
and coverage classification.

The crate accepts generic records and caller-provided bucket/group/key closures. Shogi concepts
such as SFEN, `PositionRecord`, phase, side, and evaluation buckets belong in
`shogiesa-stratify`, not here. The `cargo_toml_has_no_shogiesa_dependency` test enforces that this
crate has no `shogiesa-*` dependency.

## Modules

- `heap`: bounded top-K `HeapEntry` and `push_bounded`
- `hash`: deterministic `seeded_hash`
- `coverage`: bucket flooring, means, and `Missing`/`Under`/`Ok`/`Over` classification
- `quota`: the editable `QuotaSpec` format
- `sampling`: deterministic reservoir sampling and root-diverse quota filling

## Status

`stratifykit-core` is published on crates.io because `shogiesa-cli` depends on it. It remains in
this workspace and follows the shogiesa release cadence. A separate repository is deferred until
an independent consumer requires its own compatibility and release boundary.
