use shogiesa_core::schema::{
    MAX_SUPPORTED_SCHEMA_VERSION, MIN_SUPPORTED_SCHEMA_VERSION, PositionRecordParseError, Score,
    ScoreBound, ScorePerspective, SearchLimitKind, parse_json_line, validate_schema_version,
};

const CURRENT_FIXTURE: &str = include_str!("fixtures/schema_contract_v11.jsonl");
const LEGACY_FIXTURE: &str = include_str!("fixtures/schema_contract_v1.jsonl");

fn fixture_lines(fixture: &str) -> impl Iterator<Item = &str> {
    fixture.lines().filter(|line| !line.trim().is_empty())
}

#[test]
fn canonical_v11_fixture_round_trips_without_loss() {
    let records: Vec<_> = fixture_lines(CURRENT_FIXTURE)
        .map(|line| {
            let original: serde_json::Value = serde_json::from_str(line).unwrap();
            let record = parse_json_line(line).unwrap();
            assert_eq!(record.schema_version, MAX_SUPPORTED_SCHEMA_VERSION);
            assert_eq!(serde_json::to_value(&record).unwrap(), original);
            parse_json_line(&serde_json::to_string(&record).unwrap()).unwrap()
        })
        .collect();

    assert_eq!(records.len(), 2);
    assert!(matches!(
        records[0].observations[0].score,
        Score::Cp { value: 43 }
    ));
    assert_eq!(
        records[0].observations[0].score_perspective,
        ScorePerspective::SideToMove
    );
    assert_eq!(records[0].observations[0].score_bound, ScoreBound::Exact);
    assert_eq!(records[0].source.variation_id.as_deref(), Some("var1"));
    assert_eq!(records[0].source.branch_from_ply, Some(20));
    assert!(records[0].stability.is_some());
    assert!(records[0].game_result.is_some());

    assert!(matches!(
        records[1].observations[0].score,
        Score::Mate { moves: 5 }
    ));
    assert_eq!(
        records[1].observations[0].search_limit_kind,
        SearchLimitKind::Nodes
    );
    assert_eq!(records[1].observations[0].requested_nodes, Some(50_000));
}

#[test]
fn legacy_v1_fixture_uses_explicit_serde_defaults() {
    let record = parse_json_line(fixture_lines(LEGACY_FIXTURE).next().unwrap()).unwrap();

    assert_eq!(record.schema_version, MIN_SUPPORTED_SCHEMA_VERSION);
    assert_eq!(record.source.root_id, None);
    assert_eq!(record.source.variation_id, None);
    assert!(record.stability.is_none());
    assert_eq!(record.game_result, None);
    assert_eq!(
        record.observations[0].score_perspective,
        ScorePerspective::SideToMove
    );
    assert_eq!(record.observations[0].score_bound, ScoreBound::Exact);
    assert_eq!(
        record.observations[0].search_limit_kind,
        SearchLimitKind::Depth
    );
    assert!(!record.observations[0].was_timeout_salvaged);

    let normalized = serde_json::to_string(&record).unwrap();
    parse_json_line(&normalized).unwrap();
}

#[test]
fn unsupported_future_version_has_actionable_diagnostic() {
    let line = fixture_lines(LEGACY_FIXTURE).next().unwrap().replacen(
        "\"schema_version\":1",
        "\"schema_version\":12",
        1,
    );
    let error = parse_json_line(&line).unwrap_err();

    assert!(matches!(
        error,
        PositionRecordParseError::UnsupportedSchema(_)
    ));
    assert_eq!(
        error.to_string(),
        "unsupported shogiesa schema_version 12; supported range is 1..=11. Upgrade \
         shogiesa-core for newer records or convert the dataset explicitly"
    );
}

#[test]
fn schema_zero_is_not_treated_as_a_legacy_record() {
    let error = validate_schema_version(0).unwrap_err();
    assert_eq!(error.found, 0);
    assert_eq!(error.minimum_supported, 1);
    assert_eq!(error.maximum_supported, 11);
}

#[test]
fn malformed_json_is_reported_as_a_typed_parse_error() {
    let error = parse_json_line("not json").unwrap_err();
    assert!(matches!(error, PositionRecordParseError::Json(_)));
    assert!(
        error
            .to_string()
            .starts_with("invalid shogiesa PositionRecord JSON:")
    );
}
