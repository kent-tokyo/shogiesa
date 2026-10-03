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

## Consumer audit — 2026-10-03

The audit inspected the available checkout state, including local uncommitted changes, and pinned
the relevant files by SHA-256:

| Consumer | Revision | Relevant state |
|---|---|---|
| `quietset` | `121be2080e954d86fea63bdb478ff3a6bc37eeff` | No envelope or run-manifest contract. `schema.rs` SHA-256 `e1f38384…c50f`; stability schema SHA-256 `21a93ed3…b0eb4`. |
| `lineprior` | `29f5e315480bedd7f8a5b3606b1a0ffcb932d220` | `GateObservation.provenance` is an unvalidated string map. `gate.rs` SHA-256 `dc5495c9…5145`. |
| `veridict` | `f1948d20a9ab8927a4f68d84c252901aca1c63d4` | Strict flat 14-field envelope and manifest; unknown manifest fields are rejected. Envelope schema SHA-256 `32656617…563`. |

### Field impact

| Meaning | shogiesa flat v0 | Proposed nested v1 | Consumer impact |
|---|---|---|---|
| version | top-level producer `schema_version` | `envelope_version` plus `producer.schema_version` | veridict instead expects a flat nullable string `schema_version`; quietset has no field; lineprior can only carry a string-map entry. |
| input dataset | `dataset_sha256` | `input_dataset_sha256` | veridict compares `dataset_sha256` for drift; renaming it would break its check. |
| output dataset | absent | `output_dataset_sha256` | no audited consumer validates it. |
| engine/tool binary | `binary_sha256` | `engine_binary_sha256` plus `tool_binary_sha256` | veridict compares only flat `binary_sha256`; the other consumers do not distinguish the two identities. |
| IDs, seeds, weight, teacher manifest | flat fields | same meanings nested | lineprior can carry string forms; veridict accepts the flat fields; quietset has no run envelope. |
| validity | opaque advisory string | opaque advisory string | veridict restricts its passthrough value to `valid`/`invalid`, while its own computed validity remains separate. |

### Migration decision

Do not emit nested v1 yet. Keep the shipped flat fields unchanged because veridict actively consumes
them, quietset has no receiving contract, and lineprior does not validate the shape. Before a code
migration, the consumers must agree on version typing, input/output hash names, engine/tool binary
identity, and `validity`. Any later migration must be additive and retain flat fields plus fixtures
for at least one compatibility window.

## v0 to v1 mapping

| Shipped v0 field | Proposed v1 field | Meaning |
|---|---|---|
| `schema_version` | `producer.schema_version` | Keep the original top-level field; add a distinct producer value. |
| `dataset_sha256` | `input_dataset_sha256` | Pure rename: v0 always meant the input hash. |
| none | `output_dataset_sha256` | New; hash output after durable write. |
| `binary_sha256` | `engine_binary_sha256` | Pure rename: v0 identifies the USI engine, not shogiesa. |
| none | `tool_binary_sha256` | New; identifies the running shogiesa binary. |
| `weight_sha256`, IDs, seeds, `validity` | same names, nested | Same semantics, new location. |

The code-side migration is intentionally unscheduled. The completed audit found no compatible
consumer contract to adopt.

## If a sibling adopts it

1. Vendor the exact schema file in a normal PR.
2. Pin the canonical repository, source commit, and SHA-256 of the vendored bytes beside it.
3. Check the local pinned SHA-256 in that repository's CI. Re-vendoring is another explicit PR;
   CI must not fetch or silently update the schema.

## `validity` is advisory

`validity` is an opaque string. No repository may use it for pass/fail, promotion, or gating until
shogiesa, quietset, lineprior, and veridict agree on a vocabulary and semantics. Until then it is
only a human-readable provenance hint.
