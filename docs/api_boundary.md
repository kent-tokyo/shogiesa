# Versioned API boundary

## Public crate roles

| crate | boundary | current version marker |
|---|---|---|
| `shogiesa-core` | `schema` module, typed records, SFEN/quality helpers | `SCHEMA_VERSION = 11` |
| `shogiesa-csa` | CSA reader -> position records | workspace version |
| `shogiesa-kif` | KIF/KI2 reader -> position records | workspace version |
| `shogiesa-usi` | direct child-process USI protocol | workspace version |
| `shogiesa-pack` | derived binary encoding | `FORMAT_VERSION = 11` |
| `shogiesa-cli` | user-facing extract/label/filter/export commands | CLI `--version` |

`shogiesa-csa` and `shogiesa-kif` expose `extract_from_path` for ordinary files and
`extract_from_path_with_source` when physical and recorded source paths differ. Recursive CLI
extraction uses the latter.

## Minimal Rust flow

```rust
use shogiesa_core::{Board, SCHEMA_VERSION, schema::parse_json_line};

let board = Board::from_sfen(
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
)?;
assert_eq!(SCHEMA_VERSION, 11);
let _ = board.to_sfen();
let record = parse_json_line(json_line)?;
assert_eq!(record.schema_version, SCHEMA_VERSION);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Versioned JSONL is the stable interchange boundary. Consumers depend on `shogiesa-core`; shogiesa
does not depend on a trainer or engine internals. `schema::parse_json_line` accepts versions 1–11,
applies documented historical defaults, and rejects unsupported versions. Root-level type exports
remain available for source compatibility.

Pack is derived and returns to JSONL for inspection. USI is a child-process boundary; engine
internals are not public API.

For pack consumers, `read_header` separates bad magic, incomplete headers, and unsupported
versions. `decode` returns `UnexpectedEof` for a partial record. See
[pack compatibility](design/schema_compatibility.md#pack-error-classification).

Canonical fixtures cover the [current schema](../crates/shogiesa-core/tests/fixtures/schema_contract_v11.jsonl)
and [legacy schema 1](../crates/shogiesa-core/tests/fixtures/schema_contract_v1.jsonl). Future JSONL
schemas and non-current pack versions require an updated reader or explicit conversion.
