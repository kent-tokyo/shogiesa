//! Stable, consumer-facing JSONL schema boundary.
//!
//! Downstream training consumers should depend on `shogiesa-core` and use
//! [`parse_json_line`] instead of traversing `serde_json::Value`. The parser accepts the
//! documented historical schema range, applies the Serde defaults defined by the public types,
//! and rejects unsupported versions with an actionable error.

use thiserror::Error;

pub use crate::{
    BestMoveKind, CandidateMove, GameOutcome, GamePhase, GameResultInfo, Observation,
    PositionRecord, PositionTags, SCHEMA_VERSION, Score, ScoreBound, ScorePerspective,
    SearchLimitKind, SideToMove, SourceInfo, StabilityInfo,
};

/// Oldest JSONL schema that the current typed reader supports.
pub const MIN_SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Newest JSONL schema that the current typed reader supports.
pub const MAX_SUPPORTED_SCHEMA_VERSION: u32 = SCHEMA_VERSION;

/// An input record declares a schema version outside this reader's supported range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(
    "unsupported shogiesa schema_version {found}; supported range is {minimum_supported}..={maximum_supported}. Upgrade shogiesa-core for newer records or convert the dataset explicitly"
)]
pub struct UnsupportedSchemaVersion {
    pub found: u32,
    pub minimum_supported: u32,
    pub maximum_supported: u32,
}

/// Errors returned by [`parse_json_line`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PositionRecordParseError {
    #[error("invalid shogiesa PositionRecord JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    UnsupportedSchema(#[from] UnsupportedSchemaVersion),
}

/// Validate a JSONL schema version before a consumer uses the record.
pub fn validate_schema_version(schema_version: u32) -> Result<(), UnsupportedSchemaVersion> {
    if (MIN_SUPPORTED_SCHEMA_VERSION..=MAX_SUPPORTED_SCHEMA_VERSION).contains(&schema_version) {
        Ok(())
    } else {
        Err(UnsupportedSchemaVersion {
            found: schema_version,
            minimum_supported: MIN_SUPPORTED_SCHEMA_VERSION,
            maximum_supported: MAX_SUPPORTED_SCHEMA_VERSION,
        })
    }
}

/// Deserialize and version-check one shogiesa JSONL record.
pub fn parse_json_line(input: &str) -> Result<PositionRecord, PositionRecordParseError> {
    let record: PositionRecord = serde_json::from_str(input)?;
    record.validate_schema_version()?;
    Ok(record)
}

impl PositionRecord {
    /// Check that this record is supported by the current typed schema boundary.
    pub fn validate_schema_version(&self) -> Result<(), UnsupportedSchemaVersion> {
        validate_schema_version(self.schema_version)
    }
}
