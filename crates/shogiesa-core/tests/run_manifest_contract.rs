use std::io::Cursor;

use shogiesa_core::manifest::{
    ProvenanceStatus, RunManifestError, RunManifestV1, parse_run_manifest_v1, verify_label_run,
};

const MANIFEST: &[u8] = include_bytes!("../../../tests/fixtures/label_run_manifest_v1.json");
const INPUT: &[u8] = include_bytes!("../../../tests/fixtures/label_run_input_v1.jsonl");
const OUTPUT: &[u8] = include_bytes!("../../../tests/fixtures/label_run_output_v1.jsonl");

fn manifest() -> RunManifestV1 {
    parse_run_manifest_v1(Cursor::new(MANIFEST)).expect("canonical manifest fixture must parse")
}

#[test]
fn canonical_v1_fixture_is_typed_and_round_trips_deterministically() {
    let parsed = manifest();
    assert_eq!(parsed.validate_label().unwrap(), ProvenanceStatus::Complete);
    assert_eq!(
        parsed.extensions.get("fixture_note").unwrap(),
        "retained additive v1 field"
    );

    let encoded = serde_json::to_vec(&parsed).unwrap();
    let reparsed = parse_run_manifest_v1(Cursor::new(&encoded)).unwrap();
    assert_eq!(encoded, serde_json::to_vec(&reparsed).unwrap());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(MANIFEST).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&encoded).unwrap()
    );
}

#[test]
fn verifier_binds_the_exact_input_and_output_bytes() {
    let verified = verify_label_run(&manifest(), Cursor::new(INPUT), Cursor::new(OUTPUT)).unwrap();
    assert_eq!(verified.provenance, ProvenanceStatus::Complete);
    assert_eq!(verified.output.records, 1);
    assert_eq!(verified.output.observations_total, 2);
}

#[test]
fn verifier_rejects_tampered_input() {
    let mut input = INPUT.to_vec();
    input.push(b' ');
    let error = verify_label_run(&manifest(), Cursor::new(input), Cursor::new(OUTPUT)).unwrap_err();
    assert!(matches!(
        error,
        RunManifestError::DigestMismatch {
            field: "dataset_sha256",
            ..
        }
    ));
}

#[test]
fn verifier_rejects_tampered_output() {
    let output =
        String::from_utf8(OUTPUT.to_vec())
            .unwrap()
            .replacen("\"value\":12", "\"value\":13", 1);
    let error = verify_label_run(&manifest(), Cursor::new(INPUT), Cursor::new(output)).unwrap_err();
    assert!(matches!(
        error,
        RunManifestError::DigestMismatch {
            field: "output_sha256",
            ..
        }
    ));
}

#[test]
fn legacy_manifest_without_output_digest_is_explicitly_incomplete() {
    let mut legacy = manifest();
    legacy.output_sha256 = None;
    let verified = verify_label_run(&legacy, Cursor::new(INPUT), Cursor::new(OUTPUT)).unwrap();
    assert_eq!(
        verified.provenance,
        ProvenanceStatus::Incomplete {
            missing_digests: vec!["output_sha256"]
        }
    );
}

#[test]
fn parser_rejects_future_manifest_version() {
    let mut value: serde_json::Value = serde_json::from_slice(MANIFEST).unwrap();
    value["manifest_schema_version"] = 2.into();
    let error =
        parse_run_manifest_v1(Cursor::new(serde_json::to_vec(&value).unwrap())).unwrap_err();
    assert!(matches!(
        error,
        RunManifestError::UnsupportedManifestVersion { found: 2, .. }
    ));
}

#[test]
fn parser_rejects_missing_required_field() {
    let mut value: serde_json::Value = serde_json::from_slice(MANIFEST).unwrap();
    value.as_object_mut().unwrap().remove("command");
    assert!(matches!(
        parse_run_manifest_v1(Cursor::new(serde_json::to_vec(&value).unwrap())).unwrap_err(),
        RunManifestError::Json(_)
    ));
}

#[test]
fn validator_rejects_invalid_digest_and_contradictory_limits() {
    let mut invalid_digest = manifest();
    invalid_digest.output_sha256 = Some("not-a-sha256".to_string());
    assert!(matches!(
        invalid_digest.validate_label().unwrap_err(),
        RunManifestError::InvalidDigest {
            field: "output_sha256",
            ..
        }
    ));

    let mut contradictory = manifest();
    contradictory.nodes = Some(vec![1_000]);
    assert!(matches!(
        contradictory.validate_label().unwrap_err(),
        RunManifestError::ContradictorySearchLimits
    ));
}

#[test]
fn verifier_rejects_malformed_output_and_manifest_count_tampering() {
    let error = verify_label_run(
        &manifest(),
        Cursor::new(INPUT),
        Cursor::new(b"{not-json}\n"),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RunManifestError::InvalidOutputRecord { line: 1, .. }
    ));

    let mut wrong_counts = manifest();
    wrong_counts.observations_total = 3;
    let error =
        verify_label_run(&wrong_counts, Cursor::new(INPUT), Cursor::new(OUTPUT)).unwrap_err();
    assert!(matches!(
        error,
        RunManifestError::OutputSummaryMismatch {
            field: "observations_total",
            expected: 3,
            actual: 2
        }
    ));
}
