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

`shogiesa-csa` and `shogiesa-kif` expose `extract_from_path` for normal file provenance and
`extract_from_path_with_source` when a caller needs to open one physical path while recording a
different stable source path. The CLI uses the latter for recursive directory extraction.

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

The stable interchange boundary is versioned JSONL. Consumers depend on `shogiesa-core`; shogiesa
does not depend on Sekirei, its trainer, or engine internals. `schema::parse_json_line` is the
supported typed entry point: it accepts schema versions 1 through 11, applies explicit Serde
defaults for additive historical fields, and reports unsupported versions with the supported
range and upgrade/conversion guidance. The types remain available at the crate root for source
compatibility.

Pack is derived and must return to JSONL for inspection. USI is a process boundary: engines are
launched directly, never through shell-string interpolation, and engine-specific internals are
not part of the public API.

For pack consumers, `shogiesa-pack::read_header` reports invalid magic, incomplete headers, and
unsupported versions separately. `decode` is strict about trailing bytes: a partial record returns
`UnexpectedEof` instead of being accepted as clean EOF. See the [pack compatibility and error
classification](design/schema_compatibility.md#pack-error-classification) reference for the
contract and the corresponding unit tests.

The canonical downstream contract fixtures are
[`schema_contract_v11.jsonl`](../crates/shogiesa-core/tests/fixtures/schema_contract_v11.jsonl) and
[`schema_contract_v1.jsonl`](../crates/shogiesa-core/tests/fixtures/schema_contract_v1.jsonl). This example does not
claim that an unknown future schema is compatible. Additive historical fields use documented
Serde defaults, while future JSONL schemas and non-current pack versions require an updated reader
or explicit conversion.
