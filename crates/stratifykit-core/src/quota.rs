use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::BucketKey;

/// A hand-editable quota file: observed (or desired) per-bucket target counts. `quotas`' keys are
/// caller-defined bucket-key strings, reused verbatim (not trimmed/reparsed) so there is never a
/// second representation of "what bucket is this" that could drift from the first. `by` makes the
/// file self-describing: a caller reconstructs its own bucketing dimensions from this field alone,
/// never from separately-passed flags, so an apply-time mismatch between flags and file is
/// structurally impossible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSpec {
    // Why: kept as `stratify_format_version` (not the generic `format_version`) so the on-disk
    // JSON this crate reads/writes stays byte-for-byte compatible with shogiesa's existing
    // `stratify --write-template`/`--quota` file format.
    pub stratify_format_version: u32,
    pub input: String,
    pub by: Vec<String>,
    pub quotas: BTreeMap<BucketKey, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut quotas = BTreeMap::new();
        quotas.insert("phase:opening:".to_string(), 100);
        let spec = QuotaSpec {
            stratify_format_version: 1,
            input: "in.jsonl".to_string(),
            by: vec!["phase".to_string()],
            quotas,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: QuotaSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.quotas.get("phase:opening:"), Some(&100));
    }
}
