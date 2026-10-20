use shogiesa_core::quality_report::{
    QUALITY_REPORT_SCHEMA_VERSION, QualityReportError, parse_quality_report_v1,
};

const FIXTURE: &str = include_str!("../../../tests/fixtures/quality_report.golden");

#[test]
fn canonical_quality_report_fixture_parses_and_round_trips() {
    let report = parse_quality_report_v1(FIXTURE.as_bytes()).unwrap();
    assert_eq!(
        report.quality_report_schema_version,
        QUALITY_REPORT_SCHEMA_VERSION
    );
    assert_eq!(report.summary.accepted_records, 2);
    assert_eq!(report.summary.observations, 2);
    assert_eq!(report.summary.coverage.variations.value, 1);

    let encoded = serde_json::to_vec(&report).unwrap();
    let reparsed = parse_quality_report_v1(encoded.as_slice()).unwrap();
    assert_eq!(reparsed, report);
}

#[test]
fn future_quality_report_schema_is_rejected() {
    let future = FIXTURE.replacen(
        "\"quality_report_schema_version\": 1",
        "\"quality_report_schema_version\": 2",
        1,
    );
    let error = parse_quality_report_v1(future.as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        QualityReportError::UnsupportedVersion { found: 2, .. }
    ));
}
