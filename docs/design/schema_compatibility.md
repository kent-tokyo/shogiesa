# JSONL / pack compatibility

This table describes the on-disk boundaries implemented by shogiesa. A schema number is
provenance, not a promise that every older producer had every field listed below.

| JSONL schema | First notable addition | Current reader behavior |
|---:|---|---|
| 1 | Initial stable extraction/label dataset shape | Reads as the baseline shape |
| 2 | MultiPV policy-margin labeling | Reads baseline fields; absent newer optional fields use defaults |
| 3 | Cross-engine disagreement metadata | Reads disagreement-related fields when present |
| 4 | Complete MultiPV candidate lines | Reads candidate lists when present |
| 5 | Score-bound metadata and exact-score gates | Missing bound fields default to `exact` |
| 6 | `requested_depth` and requested-depth quality gate | Missing `requested_depth` means legacy/unknown request |
| 7 | Variation-aware `SourceInfo` (`root_id`, `variation_id`, `branch_from_ply`) | Missing variation fields default to `null` |
| 8 | Score perspective and special bestmove kind | Missing perspective defaults to `side_to_move`; missing kind is inferred where needed |
| 9 | Timeout-salvage provenance | Missing `was_timeout_salvaged` defaults to `false` |
| 10 | Game-result provenance | Missing `game_result` remains `null` |
| 11 | Fixed-node limits, telemetry, and current provenance fields | Missing additive fields use serde defaults; current writers emit schema 11 |

The JSONL reader is intentionally additive for the fields above: old records can be read and
normalized in memory, but a command that rewrites them emits the current schema. This does not
make an old record equivalent to a newly labeled record: absent request, telemetry, weight, or
game-result evidence remains absent and must not be treated as measured evidence.

## Binary pack

The pack header is `SHOGIESA` plus a little-endian `u16` format version. The current format is
11 and is checked before decoding records. `unpack` therefore accepts current format 11 only;
JSONL is the migration and inspection path for older data. A pack file is not accepted merely
because its records contain a recognizable JSON schema number, and a JSONL schema upgrade does
not silently change the binary header contract.

Pack round-trip acceptance for the current format is:

```text
JSONL (schema 11) -> pack format 11 -> JSONL (schema 11)
```

The pack encoder/decoder preserves the optional source, observation, stability, and game-result
fields represented by the current Rust types. Older pack formats need an explicit decoder or a
conversion fixture before they can be called supported; this repository does not claim that
support from the current `read_header` check alone.

### Pack error classification

The public `shogiesa-pack` API keeps format errors distinguishable by `io::ErrorKind` and a stable
diagnostic class:

| API / condition | `ErrorKind` or result | Meaning |
|---|---|---|
| `read_header`: wrong magic | `InvalidData`, `bad magic` | The file is not a shogiesa pack |
| `read_header`: short header | `UnexpectedEof` | The 10-byte header is incomplete |
| `read_header`: unsupported or wrong-endian version | `InvalidData`, `unsupported pack version N` | The header is complete but the version is not 11 little-endian |
| `decode_record`: short record | `UnexpectedEof` | A record began but did not contain all fields |
| batch `decode`: clean EOF after a complete record | `Ok(Vec<...>)` | Normal end of the pack |
| batch `decode`: incomplete trailing bytes | `UnexpectedEof` | Corrupt input; never treated as clean EOF |

`decode` is the strict batch convenience API: it buffers the input so it can distinguish clean EOF
from a partial record. Streaming callers using `read_header` and `decode_record` must perform the
same boundary check before calling `decode_record` again. The CLI `unpack` reports a partial record
as `truncated pack record` and does not claim successful output.
