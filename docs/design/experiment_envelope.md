# Experiment envelope

## Status

This is a **shogiesa-owned v1 draft**, not a shared contract. Runtime manifests still emit flat v0
fields. Do not use the draft as a gate until a sibling repository agrees on field meaning and
ownership.

The local proposal is [`schema/experiment_envelope.schema.json`](../../schema/experiment_envelope.schema.json).
Its `$id` is `urn:kent-tokyo:schema:experiment-envelope:1`. `envelope_version` describes the
envelope; a producer's `schema_version` describes its own payload.

## Rules

- `RunManifest` remains shogiesa-owned and keeps its existing top-level `schema_version`.
- A future envelope is nested under `experiment_envelope`; it does not replace manifest fields.
- IDs, seeds, `validity`, and upstream-manifest references are opaque provenance values, never
  quality or promotion gates.
- Provenance chains join separate manifests by hashes; no stage embeds or edits an upstream
  envelope.
- A migration keeps flat fields during a tested compatibility window.

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

The audit inspected the available checkout state and pinned relevant files by SHA-256:

| Consumer | Revision | Relevant state |
|---|---|---|
| `quietset` | `121be2080e954d86fea63bdb478ff3a6bc37eeff` | no envelope or run-manifest contract |
| `lineprior` | `29f5e315480bedd7f8a5b3606b1a0ffcb932d220` | provenance is an unvalidated string map |
| `veridict` | `f1948d20a9ab8927a4f68d84c252901aca1c63d4` | strict, incompatible flat envelope; unknown fields are rejected |

### Migration decision

Do not emit nested v1 yet. `veridict` consumes incompatible flat names, `quietset` has no receiver,
and `lineprior` does not validate shape. A migration remains blocked on four decisions:

- numeric versus string version fields;
- input/output dataset hash names;
- separate engine and tool binary identities; and
- a shared vocabulary for `validity`.

Any migration must be additive and retain flat fields plus fixtures for one compatibility window.

## If a sibling adopts it

1. Vendor the exact schema file in a normal PR.
2. Pin the canonical repository, source commit, and SHA-256 of the vendored bytes beside it.
3. Check the local pinned SHA-256 in that repository's CI. Re-vendoring is another explicit PR;
   CI must not fetch or silently update the schema.

## `validity` is advisory

`validity` is an opaque human-readable hint. It must not control pass/fail or promotion until the
repositories agree on its vocabulary and semantics.
