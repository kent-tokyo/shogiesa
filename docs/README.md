# Documentation

The current published release is `0.11.7`. CLI help is the option-level authority; these documents
explain contracts, interpretation, and evidence limits.

## Start here

| Need | Document |
|---|---|
| Interpret scores, stability, and quality gates | [THEORY.md](THEORY.md) |
| Consume the Rust API | [api_boundary.md](api_boundary.md) |
| Read or write position JSONL and pack files | [schema_compatibility.md](design/schema_compatibility.md) |
| Implement a streaming position JSONL consumer | [jsonl_reader_contract.md](design/jsonl_reader_contract.md) |
| Verify run manifests | [run_manifest_compatibility.md](design/run_manifest_compatibility.md) |
| Consume `quality-report` JSON | [quality_report_compatibility.md](design/quality_report_compatibility.md) |

## Build and measure datasets

| Task | Document |
|---|---|
| Record a reproducible dataset recipe | [dataset_recipe_template.md](design/dataset_recipe_template.md) |
| See completed measurements and open gates | [measurement_matrix.md](design/measurement_matrix.md) |
| Compare training recipes | [training_effect_measurement.md](design/training_effect_measurement.md) |
| Run an external opening-suite gate | [external_gate_evaluation.md](external_gate_evaluation.md) |
| Test exported priors with lineprior | [LINEPRIOR_DOGFOOD.md](LINEPRIOR_DOGFOOD.md) |

## Evidence and limits

- [Capability evidence](competitor_evidence.md) summarizes what is implemented and what remains
  unmeasured. The filename is retained for stable links; the document is not a competitor ranking.
- [Interoperability evidence](interop_evidence.md) separates local fixtures from external-tool
  claims.
- [Measurement artifacts](measurements/README.md) indexes fixed-input runs. Results do not transfer
  to other corpora, engines, hosts, or budgets without a new measurement.
- [Provenance contract decision](design/experiment_envelope.md) records why manifests remain
  producer-owned instead of sharing one cross-repository envelope.

## Releases

Use the [release checklist](release_checklist.md) for a new candidate. The current publication
evidence is [v0.11.7 release validation](release_validation_2026-10-21_v0.11.7.md).

Files named `release_validation_*.md` are historical evidence for their stated tags. Keep their
commands, hashes, failures, and claim limits unchanged; update current guidance in the documents
above instead.
