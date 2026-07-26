# Experiment envelope: canonical source, sync mechanism, and v0 -> v1 migration

**Status as of this writing**: draft v1 proposal. `schema/experiment_envelope.schema.json` in this
repo is the sole canonical source — **no schema repo exists yet**, and **no sibling repo
(quietset/lineprior/veridict) has vendored this schema under any name yet** (confirmed by checking
their default branches before writing this doc). Do not copy this schema into any sibling repo
until this v1 proposal has been reviewed and accepted there — this doc and the schema file are the
review artifact, not a fait accompli.

This is deliberately docs + a schema-JSON draft only. No shogiesa Rust code changed as part of this
document — `crates/shogiesa-cli/src/main.rs`'s `RunManifest` still emits the v0 (flat, unnested)
shape described below, unchanged. See "Migration plan" for what a future code round needs to do.

## Why this doc exists

The experiment envelope (14 shared provenance fields: IDs, artifact hashes, seeds, a validity tag)
was designed and shipped in shogiesa commit `4ab5230` as a first proposal, since none of
quietset/lineprior/veridict had an existing schema to align with. Reviewing that shipped shape
against actual cross-repo use surfaced three real ambiguities that are cheap to fix now (nothing
has vendored it yet) and expensive to fix later (once 3+ repos depend on the field names/shape):

1. `schema_version` reused as if it were the shared envelope's own version, when it was actually
   describing shogiesa's own internal data-schema version — colliding with each repo's own
   unrelated `schema_version` field once nested.
2. `dataset_sha256` documented as "consumed or produced" — ambiguous, and each pipeline stage has
   both an input and an output dataset, so one field can't mean both without a reader having to
   guess which, per manifest, from context alone.
3. `binary_sha256` conflated "the shogi engine being evaluated" with "the producer's own tool
   binary" — a real collision risk for a repo (e.g. quietset) that has a tool binary but may never
   spawn a separate shogi engine at all.

## Canonical source and versioning

- **Canonical source, for now**: `shogiesa/schema/experiment_envelope.schema.json` (this repo,
  `main` branch). Not a dedicated schema repo — standing up one now would be process overhead with
  no second maintainer yet. Revisit if/when a second repo needs write access to the schema itself,
  not before.
- **`$id`**: `urn:kent-tokyo:schema:experiment-envelope:1` — a stable, repo-independent identifier
  (a URN, not a GitHub blob URL). The prior v0 shape's `$id` pointed at a `github.com/.../blob/main/...`
  URL, which (a) changes meaning as `main` moves and (b) hard-codes "shogiesa" into an identifier
  meant to be shared *across* repos. The URN's trailing `:1` is the envelope's own major version —
  bump it (`:2`, etc.) only for a breaking change to this shared shape.
- **`envelope_version`** (a field *inside* the schema, required, `const: 1`): distinct from the
  `$id`'s version suffix. `$id` identifies "which schema document," `envelope_version` is data
  carried inside every actual JSON payload validating against it, so a consumer that already has
  the bytes in hand (not the schema doc) can still tell which version it's looking at without a
  side-channel.
- **Nesting**: this schema describes the *contents* of an `experiment_envelope` object, which a
  producer nests inside its own manifest under that key — never flattened into the manifest's own
  top level. Example shape for a manifest that already has its own top-level `schema_version`:
  ```json
  {
    "schema_version": 11,
    "command": "label",
    "...producer's own fields...": "...",
    "experiment_envelope": {
      "envelope_version": 1,
      "producer": { "name": "shogiesa", "schema_version": 11 },
      "input_dataset_sha256": "...",
      "...": "..."
    }
  }
  ```
  This is the fix for ambiguity #1 above: `schema_version` at the manifest's own top level and
  `producer.schema_version` inside the envelope can now never collide, because they're two
  different keys at two different nesting depths, even though they mean two conceptually related
  but distinct things (this manifest's own schema vs. this envelope-populating stage's schema).

## Sync mechanism (for when a sibling repo actually vendors this)

No shared Rust crate, no runtime dependency, no network access at CI time. Plain file copy +
pinned-hash verification:

1. A sibling repo vendors the file verbatim into its own tree (e.g.
   `schema/experiment_envelope.schema.json`, same relative path, for grep-ability across repos).
2. Alongside the copy, it records three values (a comment at the top of the vendored file is
   sufficient — no new file format needed):
   - `canonical_repository`: `https://github.com/kent-tokyo/shogiesa`
   - `canonical_commit_sha`: the shogiesa commit the copy was taken from
   - `schema_sha256`: SHA-256 of the vendored file's own bytes, computed at vendor time
3. That sibling's CI re-hashes its *local* vendored copy and asserts it still matches the pinned
   `schema_sha256`. This catches accidental local edits/drift of the vendored copy — it does
   **not** detect that shogiesa's canonical file has since changed (that's a separate, deliberate
   re-vendor step, not something CI should silently auto-pull). No network fetch is needed for the
   drift check itself, only for the human-initiated re-vendor.
4. Re-vendoring (adopting a newer canonical version) is a manual PR in the sibling repo: copy the
   new bytes, update the three pinned values, bump the consuming code to handle
   `envelope_version`'s new value if it changed.

## Compatibility matrix

Roles below marked **(shogiesa, confirmed)** are verified against this repo's actual code as of
commit `4ab5230`. Every other repo's row is **(presumed, unconfirmed)** — inferred only from the
pipeline's stated roles (shogiesa generates teacher observations; quietset scores stability/
reliability; lineprior predicts gate-evaluation priority; veridict makes the final judgment), since
none of those three repos were visible from the session that wrote this doc. Confirm directly
against each repo's own code before relying on any presumed row.

| Field | shogiesa (confirmed) | quietset / lineprior / veridict (presumed) |
|---|---|---|
| `envelope_version` | not yet populated (v0 code); future: `1` | same, once vendored |
| `producer.name` / `producer.schema_version` | not yet populated (v0 code has bare `schema_version` only); future: `"shogiesa"` / `SCHEMA_VERSION` | each stage sets its own identity when it writes its own manifest — not an accumulating chain (see below) |
| `experiment_id` / `candidate_id` / `baseline_id` / `lineage_id` | opaque passthrough only, orchestrator-supplied | presumably opaque passthrough everywhere; orchestrator (external to all 4 repos) is the true producer |
| `input_dataset_sha256` | **produced**: SHA-256 of `label --input` | presumably produced by each stage for its own input |
| `output_dataset_sha256` | not yet produced (v0/v1 gap, see migration plan) | presumably produced by each stage for its own output |
| `split_sha256` | opaque passthrough only | presumably produced by whichever stage performs the actual split |
| `teacher_manifest_sha256` | opaque passthrough only (cannot self-hash its own not-yet-written manifest) | presumably consumed by quietset/lineprior to trace back to a specific teacher-generation run |
| `engine_binary_sha256` | **produced**: SHA-256 of the USI engine binary `label` spawned (as `binary_sha256` in v0 code, renamed here) | presumably produced by veridict for the candidate/baseline engine binary it matches |
| `tool_binary_sha256` | not yet produced | presumably the only binary-identity field relevant to quietset if it never spawns a shogi engine directly |
| `weight_sha256` | **produced**: SHA-256 of `label --weight-file`, when given | presumably produced/consumed by veridict for the weight file under evaluation |
| `init_seed` | opaque passthrough only | presumably produced by whichever stage inits training weights |
| `split_seed` | opaque passthrough only | presumably produced by whichever stage performs the split |
| `shuffle_seed` | **produced**: `shuffle --seed` | not applicable unless a sibling repo also reorders data |
| `validity` | opaque passthrough only, `label --validity` | presumed primary producer: veridict (final-judgment stage) — **must not be used for gating anywhere until a cross-repo semantic review**, see below |

**Reconstructing the chain**: each producer's manifest carries exactly one `experiment_envelope`
object describing *that stage's own* production event — not a nested history of every upstream
stage. A tool reconstructing full provenance joins separate manifests externally, e.g. by matching
one stage's `output_dataset_sha256` against the next stage's `input_dataset_sha256`, or via
`teacher_manifest_sha256` pointing at a specific upstream `label --manifest` output.

## Migration plan: shogiesa's v0 (shipped, commit `4ab5230`) -> this v1 draft

**Nothing in shogiesa's code changes as part of this doc.** The table below is what a *future* code
round needs to do — not a promise of when. `RunManifest`'s current fields are flat, unnested v0;
they are not being deleted or renamed in code today.

| v0 field (shipped, flat on `RunManifest`) | v1 draft field (this schema) | Migration note |
|---|---|---|
| `schema_version` (bare, top-level — actually the manifest's own field, already existed before the envelope) | `experiment_envelope.producer.schema_version` | `RunManifest.schema_version` stays exactly as-is (every manifest needs its own schema version regardless of the envelope); a future round adds a *separate* nested `experiment_envelope.producer.schema_version` echoing the same value, rather than repurposing the existing top-level field. |
| `dataset_sha256` | `experiment_envelope.input_dataset_sha256` | Straight rename — shogiesa's v0 `dataset_sha256` was always the *input* hash (`hash_file_sha256(&args.input)`), never an output hash, so this is a pure rename, not a semantic change. |
| (none) | `experiment_envelope.output_dataset_sha256` | New — would require hashing `label`'s `--out` after writing, mirroring how `make-gate-openings --manifest` already computes `output_sha256` for its own `--out`. Not yet done for `label`. |
| `binary_sha256` | `experiment_envelope.engine_binary_sha256` | Straight rename — shogiesa's v0 `binary_sha256` was always the *evaluated engine's* binary (`compute_binary_sha256(&engine_path)`), never shogiesa's own tool binary, so this is a pure rename too. |
| (none) | `experiment_envelope.tool_binary_sha256` | New — would be SHA-256 of the running `shogiesa`/`shogiesa-cli` binary itself (`std::env::current_exe()`). Not yet done. |
| `weight_sha256` | `experiment_envelope.weight_sha256` | Unchanged, same field name. |
| `validity` | `experiment_envelope.validity` | Unchanged field name; tightened to explicitly forbid gating use (was already opaque passthrough in v0 code, no behavior change). |
| all other opaque passthrough fields (`experiment_id`/`candidate_id`/`baseline_id`/`lineage_id`/`split_sha256`/`teacher_manifest_sha256`/`init_seed`/`split_seed`/`shuffle_seed`) | same names, nested under `experiment_envelope` | No rename, only re-homed one nesting level deeper. |

No field is deleted from what's already shipped — every v0 name either keeps its exact meaning
under a new nested location (most fields), or is a pure rename with an unchanged underlying value
(`dataset_sha256` -> `input_dataset_sha256`, `binary_sha256` -> `engine_binary_sha256`). A future
code round implementing this migration should keep emitting the v0 flat fields for at least one
transition period if any external consumer has started depending on them, or coordinate a
synchronized cutover if not — that call is for whoever picks up the code-side migration, informed
by whether anything outside this repo has actually started reading shogiesa's manifests by then.

## `validity`: advisory only, not a gate input

`validity` remains an opaque string in v1, unchanged in type from v0. The one hard rule, made
explicit here because gating on an under-specified field is a real risk once several repos read
it: **no repo may branch a pass/fail, promotion, or gating decision on `validity`'s value until
quietset/lineprior/veridict have agreed on its vocabulary in a joint review.** Until then, treat it
as a human-readable hint only. A future, semantically-reviewed version may restructure it as an
object (e.g. `{"status": ..., "producer": ..., "reason_codes": [...]}` so a reader can tell which
repo asserted it and why) — not proposed for adoption in this v1 draft, since that structure itself
needs the same cross-repo agreement `validity`'s vocabulary does.
