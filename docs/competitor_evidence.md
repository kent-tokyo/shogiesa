# Capability evidence

This file records evidence boundaries for the `0.11.3` release. It is not a competitor
ranking.

| Area | Available evidence | Remaining limit |
|---|---|---|
| Ingestion | CSA/KIF/KI2 fixtures, malformed-prefix recovery, nested variations, recursive discovery | broad external dialect coverage is unmeasured |
| USI labeling | depth/node limits, MultiPV, bounds, timeout/restart, cache, resume, strict-protocol tests | representative multi-engine throughput is unmeasured |
| Quality and selection | stability, filter, audit, calibration, mining, balancing, distribution reports | thresholds remain corpus- and teacher-specific |
| Reproducibility | manifests, hashes, seeds, root-aware split, path/order/worker matrix | cross-tool chains still require explicit producer-owned adapters |
| Scale | clean-release 100k and 1M local resource artifacts | 10M and cross-platform measurements are unverified |
| Sekirei | pinned 0.3.68 schema check, 0.3.67/0.3.68 teacher calibration, historical fixed-node delta, and a four-game A/B smoke gate | no representative match-transfer or strength result |
| Ecosystem | Rust crates, JSONL, pack, and USI boundaries | no native GenSfen/rshogi/cshogi/rsshogi/python-shogi adapters |

The measured files are indexed in [`measurements/README.md`](measurements/README.md). Comparisons
with another tool require the same corpus, engine, search budget, hardware, and metric definition.
