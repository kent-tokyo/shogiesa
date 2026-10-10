# Provenance contract decision

## Decision

shogiesa will not emit the proposed nested `experiment_envelope`. The 2026-10-03 draft was never
a shared contract, was never emitted at runtime, and is now retired.

Each producer owns and versions its own manifest:

- shogiesa records dataset, engine, weight, split, seed, and caller-supplied correlation fields in
  its existing flat run manifests;
- lineprior owns and validates `lineprior.prior-artifact-manifest.v1` sidecars;
- veridict owns its strict run manifest;
- quietset does not currently expose a run-envelope contract.

Cross-tool orchestration must use an explicit adapter and join artifacts through exact hashes. A
consumer must validate a producer's contract with that producer's validator when one exists. It
must not copy fields into a nominally shared object and imply semantics that the producer does not
guarantee.

## Compatibility

The existing shogiesa manifest fields remain unchanged. In particular, `experiment_id`,
`candidate_id`, `baseline_id`, `lineage_id`, seeds, and advisory `validity` remain opaque
passthrough values. Dataset, engine binary, and weight hashes remain independently verifiable
SHA-256 values.

Removing the unused draft schema does not change JSONL, pack, CLI, or run-manifest output. If a
future integration needs a new field, add it to the producer that can define and verify it, then
provide a fixture-backed adapter at the repository boundary.

## Evidence reviewed

The original audit found incompatible contracts in quietset, lineprior, and veridict. Subsequent
lineprior work strengthened its own optional artifact sidecar and added `verify-artifact`; it did
not create a shared envelope. This confirms the producer-owned approach and resolves the deferred
nested-envelope roadmap item without introducing cross-repository coupling.
