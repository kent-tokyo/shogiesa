# Interoperability evidence

This table records what shogiesa can verify locally. A fixture is evidence of the named
shogiesa-side round trip; it is not evidence that an external tool accepts every field or that
the formats are semantically identical.

| boundary | local fixture/evidence | verified claim | not claimed |
|---|---|---|---|
| CSA -> JSONL | `tests/fixtures/sample.csa`, `malformed.csa` | valid prefix extraction and diagnostic skip | all CSA dialects |
| KIF/KI2 -> JSONL | parser fixtures and CLI integration tests | mainline, handicap, malformed/unterminated input, and variation provenance | arbitrary dialects or unindented nested-branch conventions |
| recursive discovery | `tests/fixtures/recursive_extract/` | sorted relative provenance, tree-wide dedup, byte-stable output, symlink non-traversal | remote filesystems or non-UTF-8 path portability |
| SFEN -> core/pack | SFEN fixtures and pack round-trip tests | SFEN records survive validation and pack/unpack | arbitrary producer-specific metadata |
| JSONL -> pack -> JSONL | pack fixtures and `shogiesa-pack` tests | current format round-trips; malformed, corrupt, unsupported, trailing, wrong-endian, and truncated bytes fail with stable diagnostics | editing pack as a primary format |
| JSONL -> USI label | `fake-usi-engine` and USI tests | direct USI boundary, limits, timeout/restart diagnostics | any particular engine's strength |
| JSONL -> external trainer | schema fixtures and consumer-owned checks | a consumer can validate the published typed record contract through an explicit adapter | generalization, match transfer, or Elo improvement |
| external result -> shogiesa report | gate-result fixture and smoke tests | complete evidence, runner/binary/weight/opening identities, and status pass-through validate without stdout scraping | a small match's Elo or release-strength conclusion |
| native external formats | no native adapter in this checkout | shared SFEN/JSONL boundary is available | native import/export compatibility without a versioned round-trip report |

The native-format row remains an explicit gap. Claim compatibility only after a round-trip or loss
report for that exact tool and version.

External checks must run against immutable inputs and retain the consumer version, command,
artifact hashes, and failure reasons. shogiesa remains the producer of its own schema and manifest;
consumer-specific validation belongs in an adapter or external evidence package.
