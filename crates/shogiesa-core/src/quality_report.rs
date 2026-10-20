//! Versioned, machine-readable dataset quality report contract.
//!
//! The report is descriptive evidence. It does not decide whether a dataset is suitable for a
//! particular trainer, engine, or strength claim. Producers may add fields to version 1; readers
//! must reject a different `quality_report_schema_version`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{GameOutcome, PositionRecord, Score, ScorePerspective, SearchLimitKind};

pub const QUALITY_REPORT_SCHEMA_VERSION: u32 = 1;
const DISTINCT_SKETCH_SIZE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityReportV1 {
    pub quality_report_schema_version: u32,
    pub shogiesa_version: String,
    pub inputs: Vec<QualityInputIdentityV1>,
    pub aggregate_decoded_sha256: String,
    pub summary: QualitySummaryV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shards: Vec<QualityShardV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityInputIdentityV1 {
    pub path: String,
    pub format: String,
    pub input_bytes: u64,
    pub input_sha256: String,
    pub decoded_bytes: u64,
    pub decoded_sha256: String,
    pub schema_versions: BTreeMap<u32, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_format_version: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<QualityManifestIdentityV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityManifestIdentityV1 {
    pub path: String,
    pub sha256: String,
    pub manifest_schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityShardV1 {
    pub input_index: usize,
    pub summary: QualitySummaryV1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualitySummaryV1 {
    pub accepted_records: u64,
    pub rejected_records: u64,
    pub rejection_reasons: BTreeMap<String, u64>,
    pub observations: u64,
    pub outcome_availability: BTreeMap<String, u64>,
    pub score_kinds: BTreeMap<String, u64>,
    pub score_perspectives: BTreeMap<String, u64>,
    pub search_limit_kinds: BTreeMap<String, u64>,
    pub requested_depth_histogram: BTreeMap<String, u64>,
    pub achieved_depth_histogram: BTreeMap<String, u64>,
    pub requested_nodes_histogram: BTreeMap<String, u64>,
    pub achieved_nodes_histogram: BTreeMap<String, u64>,
    pub requested_limit_coverage: QualityLimitCoverageV1,
    pub coverage: QualityCoverageV1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityLimitCoverageV1 {
    pub requested_present: u64,
    pub achieved_present: u64,
    pub reached_or_exceeded: u64,
    pub underreached: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityCoverageV1 {
    pub source_sequences: u64,
    pub root_sequences: u64,
    pub mainline_records: u64,
    pub variation_records: u64,
    pub games: DistinctCountV1,
    pub roots: DistinctCountV1,
    pub variations: DistinctCountV1,
    pub split_groups: DistinctCountV1,
    pub unique_positions: DistinctCountV1,
    pub unique_occurrences: DistinctCountV1,
    pub duplicate_positions: Option<u64>,
    pub duplicate_occurrences: Option<u64>,
}

/// Bounded-memory distinct count: exact through 4,096 values, deterministic KMV thereafter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistinctCountV1 {
    pub value: u64,
    pub exact: bool,
    pub algorithm: String,
}

#[derive(Debug, Error)]
pub enum QualityReportError {
    #[error("invalid shogiesa quality report JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "unsupported shogiesa quality_report_schema_version {found}; this reader supports {supported}"
    )]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("invalid shogiesa quality report: {0}")]
    InvalidSemantics(String),
}

pub fn parse_quality_report_v1(reader: impl Read) -> Result<QualityReportV1, QualityReportError> {
    let value: serde_json::Value = serde_json::from_reader(reader)?;
    let version = value
        .get("quality_report_schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            QualityReportError::InvalidSemantics(
                "quality_report_schema_version must be an unsigned 32-bit integer".to_string(),
            )
        })?;
    if version != QUALITY_REPORT_SCHEMA_VERSION {
        return Err(QualityReportError::UnsupportedVersion {
            found: version,
            supported: QUALITY_REPORT_SCHEMA_VERSION,
        });
    }
    Ok(serde_json::from_value(value)?)
}

/// Bounded-memory accumulator shared by JSONL and pack producers.
#[derive(Debug, Default)]
pub struct QualitySummaryBuilder {
    summary: QualitySummaryV1,
    games: DistinctSketch,
    roots: DistinctSketch,
    variations: DistinctSketch,
    split_groups: DistinctSketch,
    positions: DistinctSketch,
    occurrences: DistinctSketch,
    previous_source: Option<String>,
    previous_root: Option<String>,
}

impl QualitySummaryBuilder {
    /// Start a new physical shard. Sequence counts never bridge file boundaries.
    pub fn begin_shard(&mut self) {
        self.previous_source = None;
        self.previous_root = None;
    }

    pub fn reject(&mut self, reason: impl Into<String>) {
        self.summary.rejected_records += 1;
        *self
            .summary
            .rejection_reasons
            .entry(reason.into())
            .or_default() += 1;
    }

    pub fn observe(&mut self, record: &PositionRecord) {
        self.summary.accepted_records += 1;
        self.summary.observations += record.observations.len() as u64;

        let source = record.source.path.as_str();
        let root = record.source.root_id.as_deref().unwrap_or(source);
        if self.previous_source.as_deref() != Some(source) {
            self.summary.coverage.source_sequences += 1;
            self.previous_source = Some(source.to_string());
        }
        if self.previous_root.as_deref() != Some(root) {
            self.summary.coverage.root_sequences += 1;
            self.previous_root = Some(root.to_string());
        }

        self.games.insert(source.as_bytes());
        self.roots.insert(root.as_bytes());
        self.split_groups.insert(root.as_bytes());
        self.positions.insert(record.sfen.as_bytes());
        self.occurrences
            .insert(format!("{}\0{}\0{}", record.sfen, source, record.source.ply).as_bytes());
        if let Some(variation) = &record.source.variation_id {
            self.summary.coverage.variation_records += 1;
            self.variations
                .insert(format!("{root}\0{variation}").as_bytes());
        } else {
            self.summary.coverage.mainline_records += 1;
        }

        let outcome = match record.game_result.as_ref().map(|result| result.outcome) {
            None => "unavailable",
            Some(GameOutcome::BlackWins) => "black_wins",
            Some(GameOutcome::WhiteWins) => "white_wins",
            Some(GameOutcome::Draw) => "draw",
            Some(GameOutcome::Unknown) => "unknown",
        };
        increment(&mut self.summary.outcome_availability, outcome);

        for observation in &record.observations {
            increment(
                &mut self.summary.score_kinds,
                match observation.score {
                    Score::Cp { .. } => "cp",
                    Score::Mate { .. } => "mate",
                },
            );
            increment(
                &mut self.summary.score_perspectives,
                match observation.score_perspective {
                    ScorePerspective::SideToMove => "side_to_move",
                    ScorePerspective::Black => "black",
                },
            );
            increment(
                &mut self.summary.search_limit_kinds,
                match observation.search_limit_kind {
                    SearchLimitKind::Depth => "depth",
                    SearchLimitKind::Nodes => "nodes",
                },
            );
            increment(
                &mut self.summary.achieved_depth_histogram,
                depth_bucket(observation.depth),
            );

            match observation.search_limit_kind {
                SearchLimitKind::Depth => {
                    self.summary.requested_limit_coverage.achieved_present += 1;
                    if let Some(requested) = observation.requested_depth {
                        increment(
                            &mut self.summary.requested_depth_histogram,
                            depth_bucket(requested),
                        );
                        self.summary.requested_limit_coverage.requested_present += 1;
                        if observation.depth >= requested {
                            self.summary.requested_limit_coverage.reached_or_exceeded += 1;
                        } else {
                            self.summary.requested_limit_coverage.underreached += 1;
                        }
                    }
                }
                SearchLimitKind::Nodes => {
                    if let Some(achieved) = observation.nodes {
                        self.summary.requested_limit_coverage.achieved_present += 1;
                        increment(
                            &mut self.summary.achieved_nodes_histogram,
                            node_bucket(achieved),
                        );
                    }
                    if let Some(requested) = observation.requested_nodes {
                        increment(
                            &mut self.summary.requested_nodes_histogram,
                            node_bucket(requested),
                        );
                        self.summary.requested_limit_coverage.requested_present += 1;
                        match observation.nodes {
                            Some(achieved) if achieved >= requested => {
                                self.summary.requested_limit_coverage.reached_or_exceeded += 1;
                            }
                            _ => self.summary.requested_limit_coverage.underreached += 1,
                        }
                    }
                }
            }
        }
    }

    pub fn finish(mut self) -> QualitySummaryV1 {
        let games = self.games.finish();
        let roots = self.roots.finish();
        let variations = self.variations.finish();
        let split_groups = self.split_groups.finish();
        let positions = self.positions.finish();
        let occurrences = self.occurrences.finish();
        self.summary.coverage.duplicate_positions = positions.exact.then(|| {
            self.summary
                .accepted_records
                .saturating_sub(positions.value)
        });
        self.summary.coverage.duplicate_occurrences = occurrences.exact.then(|| {
            self.summary
                .accepted_records
                .saturating_sub(occurrences.value)
        });
        self.summary.coverage.games = games;
        self.summary.coverage.roots = roots;
        self.summary.coverage.variations = variations;
        self.summary.coverage.split_groups = split_groups;
        self.summary.coverage.unique_positions = positions;
        self.summary.coverage.unique_occurrences = occurrences;
        self.summary
    }
}

fn increment(map: &mut BTreeMap<String, u64>, key: impl Into<String>) {
    *map.entry(key.into()).or_default() += 1;
}

fn depth_bucket(depth: u32) -> String {
    if depth <= 255 {
        depth.to_string()
    } else {
        "256+".to_string()
    }
}

fn node_bucket(nodes: u64) -> String {
    if nodes == 0 {
        return "0".to_string();
    }
    let lower_exponent = 63 - nodes.leading_zeros();
    let lower = 1u64 << lower_exponent;
    if lower_exponent == 63 {
        format!("{lower}+")
    } else {
        let upper = (1u64 << (lower_exponent + 1)) - 1;
        format!("{lower}-{upper}")
    }
}

#[derive(Debug, Default)]
struct DistinctSketch {
    hashes: BTreeSet<[u8; 32]>,
    saturated: bool,
}

impl DistinctSketch {
    fn insert(&mut self, value: &[u8]) {
        let digest = Sha256::digest(value);
        let mut hash = [0_u8; 32];
        hash.copy_from_slice(&digest);
        if self.hashes.contains(&hash) {
            return;
        }
        if self.hashes.len() < DISTINCT_SKETCH_SIZE {
            self.hashes.insert(hash);
            return;
        }
        self.saturated = true;
        let largest = *self.hashes.last().expect("full sketch has a largest hash");
        if hash < largest {
            self.hashes.remove(&largest);
            self.hashes.insert(hash);
        }
    }

    fn finish(self) -> DistinctCountV1 {
        if !self.saturated {
            return DistinctCountV1 {
                value: self.hashes.len() as u64,
                exact: true,
                algorithm: "exact".to_string(),
            };
        }
        let largest_digest = self.hashes.last().expect("saturated sketch is non-empty");
        let largest = u64::from_be_bytes(
            largest_digest[..8]
                .try_into()
                .expect("SHA-256 prefix is 8 bytes"),
        ) as f64;
        let estimate = (((DISTINCT_SKETCH_SIZE - 1) as f64) * (u64::MAX as f64) / largest)
            .round()
            .max(DISTINCT_SKETCH_SIZE as f64) as u64;
        DistinctCountV1 {
            value: estimate,
            exact: false,
            algorithm: format!("kmv_{DISTINCT_SKETCH_SIZE}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GamePhase, PositionTags, SCHEMA_VERSION, SideToMove, SourceInfo};

    fn record(sfen: &str, source: &str) -> PositionRecord {
        PositionRecord {
            schema_version: SCHEMA_VERSION,
            sfen: sfen.to_string(),
            source: SourceInfo {
                kind: "csa".to_string(),
                path: source.to_string(),
                ply: 1,
                root_id: None,
                variation_id: None,
                branch_from_ply: None,
            },
            tags: PositionTags {
                phase: GamePhase::Opening,
                side_to_move: SideToMove::Black,
                in_check: false,
                has_capture: false,
            },
            observations: Vec::new(),
            stability: None,
            game_result: None,
        }
    }

    #[test]
    fn exact_distinct_counts_and_sequences_are_deterministic() {
        let mut builder = QualitySummaryBuilder::default();
        builder.begin_shard();
        builder.observe(&record("same", "a.csa"));
        builder.observe(&record("same", "a.csa"));
        builder.observe(&record("other", "b.csa"));
        let summary = builder.finish();
        assert_eq!(summary.accepted_records, 3);
        assert_eq!(summary.coverage.source_sequences, 2);
        assert_eq!(summary.coverage.unique_positions.value, 2);
        assert_eq!(summary.coverage.duplicate_positions, Some(1));
    }

    #[test]
    fn distinct_count_switches_to_a_bounded_kmv_estimate() {
        let mut sketch = DistinctSketch::default();
        for value in 0..=DISTINCT_SKETCH_SIZE {
            sketch.insert(value.to_string().as_bytes());
        }
        let count = sketch.finish();
        assert!(!count.exact);
        assert_eq!(count.algorithm, "kmv_4096");
        assert!(count.value >= DISTINCT_SKETCH_SIZE as u64);
    }

    #[test]
    fn future_report_version_is_rejected_before_shape_deserialization() {
        let error = parse_quality_report_v1(br#"{"quality_report_schema_version":2}"#.as_slice())
            .unwrap_err();
        assert!(matches!(
            error,
            QualityReportError::UnsupportedVersion { found: 2, .. }
        ));
    }
}
