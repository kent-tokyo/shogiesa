# Seven-axis fit evidence

This is a current implementation-fit assessment, not Elo, speed, or training-effect data. Scores
are provisional and must not be read as measured competitor rankings.

| axis | points | score | evidence | limitation |
|---|---:|---:|---|---|
| data pipeline fit | 25 | 21 | extract, label, quality, split, export CLI | no trainer |
| CSA/KIF/SFEN processing | 15 | 12 | CSA/KIF fixtures, SFEN validation, root-aware variations | external dialect coverage unmeasured |
| USI teacher labeling | 20 | 17 | depth/node, MultiPV, bounds, timeout, cache, resume | engine throughput unmeasured |
| quality diagnostics/filtering | 15 | 13 | stability, conflict, block, calibration, distribution | thresholds not universally calibrated |
| reproducibility/provenance | 10 | 9 | manifests, hashes, seeds, split/order artifacts | cross-repo envelope not adopted |
| large-scale performance | 10 | 0 | no accepted 1M/10M benchmark yet | RSS, wall time, FD and disk headroom unmeasured |
| API/ecosystem | 5 | 4 | Rust crates, JSONL, pack, USI boundaries and docs | no external bindings/adapters |
| **total** | **100** | **76** | implementation evidence only | not a competitor ranking |

The zero in performance is intentional: no speed advantage is inferred from architecture or small
fixtures. Competitor feature claims require the same fixture, engine conditions, hardware, and
measurement protocol.
