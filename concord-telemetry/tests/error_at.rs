//! Own binary: `error_at!` records an error with an explicit code location.

use concord_telemetry::{AttrValue, Attrs};

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.error_at")]
enum AtError {
    #[error("at failure")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

fn str_attr<'a>(attrs: &'a Attrs, key: &str) -> Option<&'a str> {
    match attrs.get(key) {
        Some(AttrValue::Str(text)) => Some(text),
        _ => None,
    }
}

#[test]
fn error_at_records_the_given_location_not_the_call_site() {
    let captured = concord_telemetry::testing::capture_exclusive();
    concord_telemetry::error_at!(
        "elsewhere/src/lib.rs",
        42,
        "elsewhere::inner",
        &AtError::Failed
    );
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    let record = &errors[0];
    assert_eq!(str_attr(record, "error.code"), Some("test.error_at.failed"));
    assert_eq!(
        str_attr(record, "code.file.path"),
        Some("elsewhere/src/lib.rs")
    );
    assert_eq!(record.get("code.line.number"), Some(&AttrValue::I64(42)));
    assert_eq!(
        str_attr(record, "code.module.name"),
        Some("elsewhere::inner")
    );
    // With no `source =`, the fingerprint source is the given module.
    assert_eq!(str_attr(record, "error.source"), Some("elsewhere::inner"));
}

#[test]
fn error_at_takes_source_and_fields_like_error() {
    let captured = concord_telemetry::testing::capture_exclusive();
    concord_telemetry::error_at!(
        "a.rs",
        1,
        "m",
        &AtError::Failed,
        source = "custom.source",
        runtime_session_id = "rs-1"
    );
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    assert_eq!(str_attr(&errors[0], "error.source"), Some("custom.source"));
    assert_eq!(str_attr(&errors[0], "runtime_session_id"), Some("rs-1"));
    assert_eq!(str_attr(&errors[0], "code.module.name"), Some("m"));
}

#[test]
fn error_at_anyhow_variant_exists() {
    let captured = concord_telemetry::testing::capture_exclusive();
    let err = anyhow::Error::new(AtError::Failed);
    concord_telemetry::error_at_anyhow!("a.rs", 7, "m", &err);
    assert_eq!(captured.attrs_for("error").len(), 1);
}

#[test]
fn emit_error_at_carries_runtime_fields_with_their_types() {
    let captured = concord_telemetry::testing::capture_exclusive();
    let name: &'static str = "runtime_session_id";
    let fields = [
        (name, AttrValue::Str("rs-9".to_owned())),
        ("attempt", AttrValue::I64(3)),
        ("ratio", AttrValue::F64(0.5)),
        ("retried", AttrValue::Bool(true)),
    ];
    concord_telemetry::emit_error_at(
        "hook/src/lib.rs",
        77,
        "hook::errors",
        &AtError::Failed,
        &fields,
    );
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    let record = &errors[0];
    assert_eq!(str_attr(record, "error.code"), Some("test.error_at.failed"));
    assert_eq!(str_attr(record, "runtime_session_id"), Some("rs-9"));
    assert_eq!(record.get("attempt"), Some(&AttrValue::I64(3)));
    assert_eq!(record.get("ratio"), Some(&AttrValue::F64(0.5)));
    assert_eq!(record.get("retried"), Some(&AttrValue::Bool(true)));
    assert_eq!(str_attr(record, "code.file.path"), Some("hook/src/lib.rs"));
    assert_eq!(record.get("code.line.number"), Some(&AttrValue::I64(77)));
    assert_eq!(str_attr(record, "code.module.name"), Some("hook::errors"));
    assert_eq!(str_attr(record, "error.source"), Some("hook::errors"));

    // Runtime fields do not leak into the next record.
    concord_telemetry::emit_error_at("a.rs", 1, "m", &AtError::Failed, &[]);
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 2);
    assert!(!errors[1].contains_key("attempt"));
}

#[test]
fn an_explicit_pair_wins_over_a_label_of_the_same_name() {
    let captured = concord_telemetry::testing::capture_exclusive();
    let span = tracing::info_span!("label-vs-pair");
    span.in_scope(|| {
        concord_telemetry::context::add("request_id", "from-label");
        concord_telemetry::emit_error_at(
            "a.rs",
            1,
            "m",
            &AtError::Failed,
            &[("request_id", AttrValue::Str("from-pair".to_owned()))],
        );
    });
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    assert_eq!(str_attr(&errors[0], "request_id"), Some("from-pair"));
}

#[test]
fn never_log_pair_names_are_dropped_and_the_drop_is_recorded() {
    let captured = concord_telemetry::testing::capture_exclusive();
    concord_telemetry::emit_error_at(
        "a.rs",
        1,
        "m",
        &AtError::Failed,
        &[
            ("stdout", AttrValue::Str("secret output".to_owned())),
            ("token", AttrValue::Str("t".to_owned())),
            ("attempt", AttrValue::I64(1)),
        ],
    );
    let errors = captured.errors_where(|e| e.code == "test.error_at.failed");
    assert_eq!(errors.len(), 1);
    assert!(!errors[0].fields.contains_key("stdout"));
    assert!(!errors[0].fields.contains_key("token"));
    assert_eq!(errors[0].fields.get("attempt"), Some(&AttrValue::I64(1)));
    let drops = captured.errors_where(|e| e.code == concord_telemetry::FIELD_DROPPED_CODE);
    let mut names: Vec<_> = drops
        .iter()
        .map(|e| e.fields.get("field").cloned().expect("field"))
        .collect();
    names.sort_by_key(|v| format!("{v:?}"));
    assert_eq!(
        names,
        vec![
            AttrValue::Str("stdout".to_owned()),
            AttrValue::Str("token".to_owned())
        ]
    );
    // Values are never in the drop record.
    assert!(drops
        .iter()
        .all(|e| !format!("{:?}", e.fields).contains("secret output")));
}
