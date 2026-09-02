# Interoperability evidence

This table records what shogiesa can verify locally. A fixture is evidence of the named
shogiesa-side round trip; it is not evidence that an external tool accepts every field or that
the formats are semantically identical.

| boundary | local fixture/evidence | verified claim | not claimed |
|---|---|---|---|
| CSA -> JSONL | `tests/fixtures/sample.csa`, `malformed.csa` | valid prefix extraction and diagnostic skip | all CSA dialects |
| KIF -> JSONL | `sample.kif`, `sample_handicap.kif`, `malformed.kif`, `no-terminal.kif`, `variation.kif` | mainline, handicap, malformed/unterminated input, and supported variation provenance | nested variations |
| SFEN -> core/pack | `tests/fixtures/match_position_sfen_*.txt`, pack round-trip tests | SFEN records survive validation and pack/unpack | arbitrary producer-specific metadata |
| JSONL -> pack -> JSONL | `tests/fixtures/pack_input.jsonl`, `malformed_mixed.jsonl`, `pack_bad_magic.hex`, `pack_truncated_header.hex`, `pack_unsupported_version.hex`, `pack_trailing_bytes.hex`, `pack_wrong_endian_version.hex`, `shogiesa-pack` tests, and CLI schema fixtures v1-v11 | current format 11 round-trips through JSONL; malformed input is counted and corrupt/unsupported/trailing/wrong-endian bytes fail; pack manifest records artifact hash and counts | editing pack as a primary format |
| JSONL -> USI label | `fake-usi-engine` and USI tests | direct USI boundary, limits, timeout/restart diagnostics | any particular engine's strength |
| GenSfen/rshogi/cshogi/rsshogi/python-shogi | no external adapter in this checkout | only shared SFEN/JSONL boundary is proposed | native import/export compatibility |

The external-tool row remains an explicit measurement gap. “Compatible” may be claimed only after
an external round-trip or a loss report is recorded for that tool.
