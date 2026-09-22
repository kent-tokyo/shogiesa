# Experiment envelope

## Status

This is a **shogiesa-owned v1 draft**, not a cross-repository contract. Runtime manifests still
emit the backward-compatible flat v0 fields. Do not vendor this schema into another repository or
use it as a gate until at least one sibling repository agrees on field meaning and ownership.

The local proposal is [`schema/experiment_envelope.schema.json`](../../schema/experiment_envelope.schema.json).
Its `$id` is `urn:kent-tokyo:schema:experiment-envelope:1`; `envelope_version: 1` is the version
carried in a payload. The two versions serve different purposes and must not be conflated with a
producer's own manifest `schema_version`.

## Rules

- `RunManifest` remains shogiesa-owned and keeps its existing top-level `schema_version`.
- A future envelope is nested under `experiment_envelope`; it does not replace manifest fields.
- IDs, seeds, `validity`, and upstream-manifest references are opaque provenance values, never
  quality or promotion gates.
- Provenance chains join separate manifests by hashes; no stage embeds or edits an upstream
  envelope.
- A migration preserves the existing flat fields for a transition period and adds compatibility
  tests before any consumer relies on v1.

Example of the proposed nesting:

```json
{
  "schema_version": 11,
  "command": "label",
  "experiment_envelope": {
    "envelope_version": 1,
    "producer": { "name": "shogiesa", "schema_version": 11 },
    "input_dataset_sha256": "..."
  }
}
```

## What is known

| Source | State | Consequence |
|---|---|---|
| shogiesa | flat v0 fields are shipped | v1 is documentation/schema only; no nested object is emitted. |
| `veridict` checkout | separately maintained flat schema | materially different: no envelope/producer version, different hash names, explicit nulls, unknown fields rejected. |
| `quietset`, `lineprior` | checkout unavailable during review | compatibility is unmeasured. |

Therefore neither the shogiesa draft nor the veridict shape is canonical across repositories.

## v0 to v1 mapping

| Shipped v0 field | Proposed v1 field | Meaning |
|---|---|---|
| `schema_version` | `producer.schema_version` | Keep the original top-level field; add a distinct producer value. |
| `dataset_sha256` | `input_dataset_sha256` | Pure rename: v0 always meant the input hash. |
| none | `output_dataset_sha256` | New; hash output after durable write. |
| `binary_sha256` | `engine_binary_sha256` | Pure rename: v0 identifies the USI engine, not shogiesa. |
| none | `tool_binary_sha256` | New; identifies the running shogiesa binary. |
| `weight_sha256`, IDs, seeds, `validity` | same names, nested | Same semantics, new location. |

The code-side migration is intentionally unscheduled. It requires a consumer decision first.

## If a sibling adopts it

1. Vendor the exact schema file in a normal PR.
2. Pin the canonical repository, source commit, and SHA-256 of the vendored bytes beside it.
3. Check the local pinned SHA-256 in that repository's CI. Re-vendoring is another explicit PR;
   CI must not fetch or silently update the schema.

## `validity` is advisory

`validity` is an opaque string. No repository may use it for pass/fail, promotion, or gating until
shogiesa, quietset, lineprior, and veridict agree on a vocabulary and semantics. Until then it is
only a human-readable provenance hint.
