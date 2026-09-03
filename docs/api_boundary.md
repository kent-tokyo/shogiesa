# Versioned API boundary

## Public crate roles

| crate | boundary | current version marker |
|---|---|---|
| `shogiesa-core` | typed `PositionRecord`, `Observation`, SFEN/quality helpers | `SCHEMA_VERSION = 11` |
| `shogiesa-csa` | CSA reader -> position records | workspace version |
| `shogiesa-kif` | KIF/KI2 reader -> position records | workspace version |
| `shogiesa-usi` | direct child-process USI protocol | workspace version |
| `shogiesa-pack` | derived binary encoding | `FORMAT_VERSION = 11` |
| `shogiesa-cli` | user-facing extract/label/filter/export commands | CLI `--version` |

## Minimal Rust flow

```rust
use shogiesa_core::{Board, PositionRecord, SCHEMA_VERSION};

let board = Board::from_sfen(
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
)?;
assert_eq!(SCHEMA_VERSION, 11);
let _ = board.to_sfen();
let _record: PositionRecord = serde_json::from_str(json_line)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The stable interchange boundary is versioned JSONL. Pack is derived and must return to JSONL for
inspection. USI is a process boundary: engines are launched directly, never through shell-string
interpolation, and engine-specific internals are not part of the public API.

For pack consumers, `shogiesa-pack::read_header` reports invalid magic, incomplete headers, and
unsupported versions separately. `decode` is strict about trailing bytes: a partial record returns
`UnexpectedEof` instead of being accepted as clean EOF. See the [pack compatibility and error
classification](design/schema_compatibility.md#pack-error-classification) reference for the
contract and the corresponding unit tests.

This example documents the API shape; it is not a claim that every future schema version preserves
all fields unchanged. Additive JSONL fields use serde defaults where documented, while pack readers
accept the current format version only.
