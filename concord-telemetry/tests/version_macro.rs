#[test]
fn pick_prefers_a_non_empty_sha_over_the_package_version() {
    assert_eq!(
        concord_telemetry::version!(@pick Some("abc123-dirty"), "0.1.0"),
        "abc123-dirty"
    );
    assert_eq!(
        concord_telemetry::version!(@pick None::<&str>, "0.1.0"),
        "0.1.0"
    );
    assert_eq!(
        concord_telemetry::version!(@pick Some(""), "0.1.0"),
        "0.1.0"
    );
}

#[test]
fn version_reads_the_callers_build_environment() {
    let expected = match option_env!("CONCORD_GIT_SHA") {
        Some(sha) if !sha.is_empty() => sha,
        _ => env!("CARGO_PKG_VERSION"),
    };
    assert_eq!(concord_telemetry::version!(), expected);
}
