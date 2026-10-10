# Run manifest compatibility

`--manifest` outputs use `manifest_schema_version` to version the manifest
object itself. This is independent from:

- `schema_version`, which versions each `PositionRecord` in JSONL data;
- `pack_format_version`, which versions the binary pack header; and
- `shogiesa_version`, which identifies the producing CLI release.

The current run-manifest version is **1**. Consumers may accept new optional
fields added within version 1. Removing or renaming a field, changing a field's
type, or changing its meaning requires a new manifest schema version. Unknown
future versions must fail closed unless the consumer explicitly supports them.

## Required fields in version 1

Every manifest emitted through the shared `RunManifest` path contains:

- `manifest_schema_version`, `shogiesa_version`, `git_sha`;
- `schema_version`, `pack_format_version`;
- `command`, `args`, `input_path`, `input_hash`, `fingerprint_algorithm`;
- record counts: `records_read`, `records_kept`, `records_dropped`,
  `labeled_records`, `unlabeled_records`, `observations_with_candidates`,
  `observations_total`, `requested_depth_total`, and
  `requested_depth_underreach`.

Empty maps and command-specific fields may be omitted. For `label`, fields such
as `engine_name`, exactly one of `depths` or `nodes`, `multipv`,
`engine_options`, `jobs`, cache/timeout counts, and engine provenance describe
the requested and observed teacher run. A consumer must validate the fields it
needs instead of assuming every command emits label-only fields.

The canonical label example is
[`tests/fixtures/label_run_manifest_v1.json`](../../tests/fixtures/label_run_manifest_v1.json).
It is an inspectable contract fixture, not evidence from an actual teacher run.
