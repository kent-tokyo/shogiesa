# Machine-readable quality report

`shogiesa quality-report` emits producer-side evidence for JSONL and versioned pack files. It is
descriptive: it does not run an engine, relabel data, choose weights, or claim training quality or
playing strength.

```bash
shogiesa quality-report --input train.jsonl --out quality.json
shogiesa quality-report --input train.shgpk --out packed-quality.json
```

Repeat `--input` for ordered shards. Repeat `--manifest` exactly once per input to bind each shard
to a run-manifest identity. `--include-shards` includes per-input summaries as well as the global
summary.

## Version and identity

`quality_report_schema_version` is currently `1`. It changes independently from position schema,
pack format, and run-manifest versions. Rust consumers should use
`shogiesa_core::quality_report::parse_quality_report_v1`; unknown breaking versions are rejected.

Each input records:

- format, byte length, and SHA-256 of the exact source bytes;
- SHA-256 and byte length of accepted records serialized as canonical compact JSONL;
- observed position-schema versions and, for pack input, its format version;
- optional run-manifest path, SHA-256, and manifest schema version.

The decoded digest makes equivalent current-schema JSONL and pack content comparable while the
input digest keeps their different transport bytes visible. Input order is part of a multi-shard
report's configuration and aggregate decoded digest.

## Semantic summary

The v1 summary contains accepted/rejected records, stable rejection reasons, outcomes, score kinds
and perspectives, search-limit kinds, requested/achieved depth and node histograms, and limit
underreach. Coverage includes contiguous source/root sequences, mainline and variation records,
games, roots, variations, split groups, unique SFENs, and unique occurrence keys.

Depth histograms use exact keys through 255 and `256+` thereafter. Node histograms use power-of-two
ranges. Their key space is bounded.

Distinct counts are exact through 4,096 values. Larger cardinalities use a deterministic 4,096-item
KMV sketch and set `exact: false`; duplicate counts are then `null`. This bounds memory independently
of corpus size. Additive shard counts compose by summation; global distinct counts are computed over
the complete ordered stream so duplicates crossing shard boundaries are counted once.

## Rejection reasons

Stable v1 keys are:

| key | meaning |
|---|---|
| `json_invalid_record` | malformed JSON or a typed record shape error |
| `json_unsupported_schema` | position schema is outside the supported range |
| `json_other_parse_error` | a newer typed-reader parse class unknown to this CLI |
| `pack_truncated_header` | fewer than ten pack-header bytes |
| `pack_bad_magic` | header magic is not `SHOGIESA` |
| `pack_unsupported_version` | pack version is not supported by this binary |
| `pack_truncated_record` | a record ends before all fields are decoded |
| `pack_invalid_record` | a framed field has an invalid tag or encoding |

Pack records are variable-length and have no recovery delimiter. After a corrupt pack record, the
report stops that shard and records one rejection instead of guessing the next boundary.

The canonical v1 fixture is
[`tests/fixtures/quality_report.golden`](../../tests/fixtures/quality_report.golden). JSONL/pack
equivalence, malformed/future JSONL, manifest identity, shard aggregation, and pack corruption are
covered by CLI tests.
