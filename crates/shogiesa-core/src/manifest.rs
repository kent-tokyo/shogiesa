//! Typed, consumer-facing run-manifest contract and label artifact verifier.
//!
//! Manifest schema versioning is independent from [`crate::SCHEMA_VERSION`]. Consumers should
//! parse manifests with [`parse_run_manifest_v1`] and verify label artifacts with
//! [`verify_label_run`], which hashes and parses the output through the same opened stream.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::schema::{PositionRecordParseError, parse_json_line, validate_schema_version};
use crate::{GamePhase, PositionRecord, QualityConfig, requested_depth_underreached};

/// Current version of the run-manifest object.
pub const RUN_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Version 1 of the manifest emitted by commands supporting `--manifest`.
///
/// Unknown additive fields are retained in [`RunManifestV1::extensions`], so a v1 consumer can
/// read and re-serialize manifests produced by a newer compatible shogiesa release without
/// silently discarding those fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifestV1 {
    pub manifest_schema_version: u32,
    pub shogiesa_version: String,
    pub git_sha: String,
    pub schema_version: u32,
    pub pack_format_version: u16,
    pub command: String,
    pub args: Vec<String>,
    pub input_path: String,
    pub input_hash: String,
    pub fingerprint_algorithm: String,
    pub records_read: usize,
    pub records_kept: usize,
    pub records_dropped: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub drop_reasons: BTreeMap<String, usize>,
    pub labeled_records: usize,
    pub unlabeled_records: usize,
    pub observations_with_candidates: usize,
    pub observations_total: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub score_bound_distribution: BTreeMap<String, usize>,
    pub requested_depth_total: usize,
    pub requested_depth_underreach: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_config: Option<QualityConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depths: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multipv: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jobs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_launch_failures: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_salvaged_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hits: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_misses: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_fingerprint_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records_per_sec: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_engine_time_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_order: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_root_share_in_any_bucket: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinct_roots_kept: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teacher_manifest_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_threads: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_hash_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_violations_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_restarts: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_valid_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root_distribution: Option<BTreeMap<String, usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_distribution: Option<BTreeMap<String, usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_distribution: Option<BTreeMap<String, usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_distribution: Option<BTreeMap<String, usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_distribution: Option<BTreeMap<GamePhase, usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_distribution: Option<BTreeMap<u32, usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_algorithm_version: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

impl RunManifestV1 {
    /// Create the common manifest shape. Command-specific producers fill their optional fields.
    pub fn new(
        command: impl Into<String>,
        input_path: &Path,
        pack_format_version: u16,
        shogiesa_version: impl Into<String>,
        git_sha: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            manifest_schema_version: RUN_MANIFEST_SCHEMA_VERSION,
            shogiesa_version: shogiesa_version.into(),
            git_sha: git_sha.into(),
            schema_version: crate::SCHEMA_VERSION,
            pack_format_version,
            command: command.into(),
            args,
            input_path: input_path.display().to_string(),
            input_hash: String::new(),
            fingerprint_algorithm: "blake3".to_string(),
            records_read: 0,
            records_kept: 0,
            records_dropped: 0,
            drop_reasons: BTreeMap::new(),
            labeled_records: 0,
            unlabeled_records: 0,
            observations_with_candidates: 0,
            observations_total: 0,
            score_bound_distribution: BTreeMap::new(),
            requested_depth_total: 0,
            requested_depth_underreach: 0,
            filter_config: None,
            engine_name: None,
            depths: None,
            multipv: None,
            engine_options: None,
            jobs: None,
            engine_launch_failures: None,
            timeout_salvaged_count: None,
            cache_hits: None,
            cache_misses: None,
            engine_fingerprint_mode: None,
            records_per_sec: None,
            cache_hit_rate: None,
            average_engine_time_ms: None,
            preserve_order: None,
            resume_from: None,
            resumed_count: None,
            max_root_share_in_any_bucket: None,
            distinct_roots_kept: None,
            order_hash: None,
            experiment_id: None,
            nodes: None,
            candidate_id: None,
            baseline_id: None,
            lineage_id: None,
            dataset_sha256: None,
            split_sha256: None,
            teacher_manifest_sha256: None,
            binary_sha256: None,
            weight_sha256: None,
            init_seed: None,
            split_seed: None,
            shuffle_seed: None,
            validity: None,
            engine_threads: None,
            engine_hash_mb: None,
            protocol_violations_count: None,
            engine_restarts: None,
            selection_seed: None,
            canonical_valid_count: None,
            output_sha256: None,
            source_root_distribution: None,
            engine_distribution: None,
            weight_distribution: None,
            source_distribution: None,
            phase_distribution: None,
            material_distribution: None,
            selection_algorithm_version: None,
            extensions: BTreeMap::new(),
        }
    }

    /// Validate the v1 label-specific invariants without opening artifact files.
    pub fn validate_label(&self) -> Result<ProvenanceStatus, RunManifestError> {
        if self.manifest_schema_version != RUN_MANIFEST_SCHEMA_VERSION {
            return Err(RunManifestError::UnsupportedManifestVersion {
                found: self.manifest_schema_version,
                supported: RUN_MANIFEST_SCHEMA_VERSION,
            });
        }
        if self.command != "label" {
            return Err(RunManifestError::WrongCommand(self.command.clone()));
        }
        validate_schema_version(self.schema_version)
            .map_err(|error| RunManifestError::InvalidSemantics(error.to_string()))?;

        match (&self.depths, &self.nodes) {
            (Some(depths), None) => validate_limits("depths", depths.iter().copied())?,
            (None, Some(nodes)) => validate_limits("nodes", nodes.iter().copied())?,
            _ => return Err(RunManifestError::ContradictorySearchLimits),
        }
        if self.jobs == Some(0) {
            return Err(RunManifestError::InvalidSemantics(
                "label jobs must be greater than zero".to_string(),
            ));
        }
        if self.multipv == Some(0) {
            return Err(RunManifestError::InvalidSemantics(
                "label multipv must be greater than zero".to_string(),
            ));
        }
        validate_count_partition(
            "records_read",
            self.records_read,
            "records_kept",
            self.records_kept,
            "records_dropped",
            self.records_dropped,
        )?;
        validate_count_partition(
            "records_kept",
            self.records_kept,
            "labeled_records",
            self.labeled_records,
            "unlabeled_records",
            self.unlabeled_records,
        )?;
        if self.observations_with_candidates > self.observations_total {
            return Err(RunManifestError::InvalidSemantics(
                "observations_with_candidates exceeds observations_total".to_string(),
            ));
        }
        if self.requested_depth_total > self.observations_total
            || self.requested_depth_underreach > self.requested_depth_total
        {
            return Err(RunManifestError::InvalidSemantics(
                "requested-depth counts contradict observation counts".to_string(),
            ));
        }

        for (field, value) in [
            ("dataset_sha256", self.dataset_sha256.as_deref()),
            ("output_sha256", self.output_sha256.as_deref()),
            ("binary_sha256", self.binary_sha256.as_deref()),
            ("weight_sha256", self.weight_sha256.as_deref()),
            ("split_sha256", self.split_sha256.as_deref()),
            (
                "teacher_manifest_sha256",
                self.teacher_manifest_sha256.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                validate_sha256(field, value)?;
            }
        }

        let mut missing_digests = Vec::new();
        if self.dataset_sha256.is_none() {
            missing_digests.push("dataset_sha256");
        }
        if self.output_sha256.is_none() {
            missing_digests.push("output_sha256");
        }
        Ok(if missing_digests.is_empty() {
            ProvenanceStatus::Complete
        } else {
            ProvenanceStatus::Incomplete { missing_digests }
        })
    }
}

fn validate_count_partition(
    total_name: &'static str,
    total: usize,
    first_name: &'static str,
    first: usize,
    second_name: &'static str,
    second: usize,
) -> Result<(), RunManifestError> {
    if first.checked_add(second) != Some(total) {
        return Err(RunManifestError::InvalidSemantics(format!(
            "{first_name} + {second_name} must equal {total_name} ({total})"
        )));
    }
    Ok(())
}

fn validate_limits<T>(
    field: &'static str,
    values: impl Iterator<Item = T>,
) -> Result<(), RunManifestError>
where
    T: Copy + Ord + Default,
{
    let values: Vec<T> = values.collect();
    let zero = T::default();
    let distinct: BTreeSet<T> = values.iter().copied().collect();
    if values.is_empty() || values.contains(&zero) || distinct.len() != values.len() {
        return Err(RunManifestError::InvalidSearchLimits { field });
    }
    Ok(())
}

fn validate_sha256(field: &'static str, value: &str) -> Result<(), RunManifestError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RunManifestError::InvalidDigest {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

/// Whether a verified legacy label manifest carried every digest needed for an exact binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceStatus {
    Complete,
    Incomplete { missing_digests: Vec<&'static str> },
}

/// Aggregate values recomputed from the exact output JSONL stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelOutputSummary {
    pub records: usize,
    pub labeled_records: usize,
    pub unlabeled_records: usize,
    pub observations_with_candidates: usize,
    pub observations_total: usize,
    pub requested_depth_total: usize,
    pub requested_depth_underreach: usize,
}

/// Evidence returned after checking a label manifest against opened input/output streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRunVerification {
    pub provenance: ProvenanceStatus,
    pub dataset_sha256: String,
    pub output_sha256: String,
    pub output: LabelOutputSummary,
}

/// Typed run-manifest and artifact-verification failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RunManifestError {
    #[error("invalid shogiesa run manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cannot read shogiesa run artifact: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "unsupported shogiesa manifest_schema_version {found}; this reader supports {supported}"
    )]
    UnsupportedManifestVersion { found: u32, supported: u32 },
    #[error("expected a label run manifest, found command {0:?}")]
    WrongCommand(String),
    #[error("label manifest must contain exactly one non-empty, positive depths or nodes list")]
    ContradictorySearchLimits,
    #[error("label manifest {field} must be non-empty, positive, and distinct")]
    InvalidSearchLimits { field: &'static str },
    #[error(
        "invalid {field} SHA-256 digest {value:?}; expected 64 lowercase hexadecimal characters"
    )]
    InvalidDigest { field: &'static str, value: String },
    #[error("invalid label manifest semantics: {0}")]
    InvalidSemantics(String),
    #[error("invalid output PositionRecord at line {line}: {source}")]
    InvalidOutputRecord {
        line: usize,
        #[source]
        source: PositionRecordParseError,
    },
    #[error("{field} mismatch: manifest={expected}, artifact={actual}")]
    DigestMismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("output summary mismatch for {field}: manifest={expected}, artifact={actual}")]
    OutputSummaryMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
}

/// Parse a run manifest and reject unknown breaking versions before deserializing the v1 shape.
pub fn parse_run_manifest_v1(reader: impl Read) -> Result<RunManifestV1, RunManifestError> {
    let value: Value = serde_json::from_reader(reader)?;
    let found = value
        .get("manifest_schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            RunManifestError::InvalidSemantics(
                "manifest_schema_version must be an unsigned integer".to_string(),
            )
        })?;
    if found != RUN_MANIFEST_SCHEMA_VERSION {
        return Err(RunManifestError::UnsupportedManifestVersion {
            found,
            supported: RUN_MANIFEST_SCHEMA_VERSION,
        });
    }
    Ok(serde_json::from_value(value)?)
}

/// Verify a label manifest against already-open input and output streams.
///
/// The output stream is parsed and hashed in one pass, closing the hash-then-reopen race. Missing
/// digests in an older v1 manifest produce [`ProvenanceStatus::Incomplete`], never a false claim
/// that exact-byte verification succeeded.
pub fn verify_label_run(
    manifest: &RunManifestV1,
    mut input: impl Read,
    output: impl Read,
) -> Result<LabelRunVerification, RunManifestError> {
    let provenance = manifest.validate_label()?;

    let mut input_hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        input_hasher.update(&buffer[..read]);
    }
    let dataset_sha256 = hex_digest(input_hasher.finalize());

    let (output_sha256, output_summary) = scan_label_output(output)?;
    check_digest(
        "dataset_sha256",
        manifest.dataset_sha256.as_deref(),
        &dataset_sha256,
    )?;
    check_digest(
        "output_sha256",
        manifest.output_sha256.as_deref(),
        &output_sha256,
    )?;

    for (field, expected, actual) in [
        (
            "records_kept",
            manifest.records_kept,
            output_summary.records,
        ),
        (
            "labeled_records",
            manifest.labeled_records,
            output_summary.labeled_records,
        ),
        (
            "unlabeled_records",
            manifest.unlabeled_records,
            output_summary.unlabeled_records,
        ),
        (
            "observations_with_candidates",
            manifest.observations_with_candidates,
            output_summary.observations_with_candidates,
        ),
        (
            "observations_total",
            manifest.observations_total,
            output_summary.observations_total,
        ),
        (
            "requested_depth_total",
            manifest.requested_depth_total,
            output_summary.requested_depth_total,
        ),
        (
            "requested_depth_underreach",
            manifest.requested_depth_underreach,
            output_summary.requested_depth_underreach,
        ),
    ] {
        if expected != actual {
            return Err(RunManifestError::OutputSummaryMismatch {
                field,
                expected,
                actual,
            });
        }
    }

    Ok(LabelRunVerification {
        provenance,
        dataset_sha256,
        output_sha256,
        output: output_summary,
    })
}

fn check_digest(
    field: &'static str,
    expected: Option<&str>,
    actual: &str,
) -> Result<(), RunManifestError> {
    if let Some(expected) = expected
        && expected != actual
    {
        return Err(RunManifestError::DigestMismatch {
            field,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn finalize(self) -> String {
        hex_digest(self.hasher.finalize())
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        Ok(read)
    }
}

fn scan_label_output(output: impl Read) -> Result<(String, LabelOutputSummary), RunManifestError> {
    let mut reader = BufReader::new(HashingReader::new(output));
    let mut summary = LabelOutputSummary {
        records: 0,
        labeled_records: 0,
        unlabeled_records: 0,
        observations_with_candidates: 0,
        observations_total: 0,
        requested_depth_total: 0,
        requested_depth_underreach: 0,
    };
    let mut line = String::new();
    let mut line_number = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_number += 1;
        if line.trim().is_empty() {
            continue;
        }
        let record =
            parse_json_line(&line).map_err(|source| RunManifestError::InvalidOutputRecord {
                line: line_number,
                source,
            })?;
        accumulate_output_summary(&record, &mut summary);
    }
    let digest = reader.into_inner().finalize();
    Ok((digest, summary))
}

fn accumulate_output_summary(record: &PositionRecord, summary: &mut LabelOutputSummary) {
    summary.records += 1;
    if record.observations.is_empty() {
        summary.unlabeled_records += 1;
    } else {
        summary.labeled_records += 1;
    }
    for observation in &record.observations {
        summary.observations_total += 1;
        if !observation.candidates.is_empty() {
            summary.observations_with_candidates += 1;
        }
        if observation.requested_depth.is_some() {
            summary.requested_depth_total += 1;
            if requested_depth_underreached(observation) {
                summary.requested_depth_underreach += 1;
            }
        }
    }
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
