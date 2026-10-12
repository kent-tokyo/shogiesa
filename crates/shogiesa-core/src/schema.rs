//! Stable, consumer-facing JSONL schema boundary.
//!
//! Downstream training consumers should depend on `shogiesa-core` and use
//! [`parse_json_line`] instead of traversing `serde_json::Value`. The parser accepts the
//! documented historical schema range, applies the Serde defaults defined by the public types,
//! and rejects unsupported versions with an actionable error.

use std::io::{self, BufRead};

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

/// One typed record yielded by [`PositionRecordJsonlReader`].
#[derive(Debug, Clone)]
pub struct PositionRecordLine {
    /// One-based physical line number in the input stream. Skipped blank lines still count.
    pub line_number: usize,
    /// Parsed, schema-validated position record from this line.
    pub record: PositionRecord,
}

/// A line-numbered failure yielded by [`PositionRecordJsonlReader`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PositionRecordLineError {
    /// The input stream could not be read. Iteration terminates after this error.
    #[error("failed to read shogiesa JSONL at physical line {line_number}: {source}")]
    Read {
        line_number: usize,
        #[source]
        source: io::Error,
    },
    /// A complete line was read but did not contain a supported position record.
    #[error("invalid shogiesa PositionRecord at physical line {line_number}: {source}")]
    Parse {
        line_number: usize,
        #[source]
        source: PositionRecordParseError,
    },
}

impl PositionRecordLineError {
    /// One-based physical line number where the failure occurred.
    pub fn line_number(&self) -> usize {
        match self {
            Self::Read { line_number, .. } | Self::Parse { line_number, .. } => *line_number,
        }
    }
}

/// Incremental, bounded-memory reader for shogiesa position JSONL.
///
/// Blank or whitespace-only lines are skipped but still contribute to physical line numbers.
/// Parse and schema errors consume only the failing line, so callers can stop or continue by
/// deciding whether to request the next item. A read error terminates the iterator.
pub struct PositionRecordJsonlReader<R> {
    reader: R,
    line: String,
    line_number: usize,
    finished: bool,
}

impl<R: BufRead> PositionRecordJsonlReader<R> {
    /// Create a streaming reader at physical line zero.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            line: String::new(),
            line_number: 0,
            finished: false,
        }
    }

    /// Return the wrapped reader, preserving its current stream position.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R: BufRead> Iterator for PositionRecordJsonlReader<R> {
    type Item = Result<PositionRecordLine, PositionRecordLineError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            self.line.clear();
            let next_line_number = self.line_number.saturating_add(1);
            match self.reader.read_line(&mut self.line) {
                Ok(0) => {
                    self.finished = true;
                    return None;
                }
                Ok(_) => {
                    self.line_number = next_line_number;
                    if self.line.trim().is_empty() {
                        continue;
                    }
                    return Some(
                        parse_json_line(&self.line)
                            .map(|record| PositionRecordLine {
                                line_number: self.line_number,
                                record,
                            })
                            .map_err(|source| PositionRecordLineError::Parse {
                                line_number: self.line_number,
                                source,
                            }),
                    );
                }
                Err(source) => {
                    self.finished = true;
                    return Some(Err(PositionRecordLineError::Read {
                        line_number: next_line_number,
                        source,
                    }));
                }
            }
        }
    }
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
