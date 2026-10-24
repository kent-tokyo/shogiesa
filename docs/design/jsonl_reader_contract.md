# Position JSONL reader contract

All normal `PositionRecord` streams pass through
`shogiesa_core::schema::PositionRecordJsonlReader`. It preserves physical line numbers, skips
blank lines, rejects unsupported schema versions, and keeps parse errors recoverable so a
permissive CLI can diagnose and count them.

The CLI has two entry points:

- `open_position_jsonl`: typed streaming without input identity collection.
- `open_hashed_position_jsonl`: the same typed stream plus normalized-line blake3 and exact-byte
  SHA-256 identities from one open file.

The normalized digest retains the historical manifest rule: remove one line ending and hash the
line followed by `\n`, including blank and final unterminated lines. The exact digest hashes bytes
as read. Commands must not reopen a streaming input merely to compute either identity.

## Deliberate exceptions

- `validate` first parses `serde_json::Value`, then calls `parse_json_line`. This distinguishes
  malformed JSON from a valid JSON value with an invalid record shape or unsupported schema.
- `label --resume-from` builds a key-only byte-offset index without allocating observations. A
  selected line is schema-checked with `parse_json_line` before its observations are reused.
- Cache entries, quota files, presets, manifests, and recipe files are separate JSON contracts,
  not position JSONL, and use their own typed parsers.

No production path may deserialize a complete `PositionRecord` directly with `serde_json`. New
position consumers use the typed reader or document and test why raw bytes are required.
