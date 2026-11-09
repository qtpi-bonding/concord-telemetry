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

#[test]
fn repo_pick_handles_set_unset_and_empty() {
    assert_eq!(concord_telemetry::repo!(@pick Some("grorg")), "grorg");
    assert_eq!(concord_telemetry::repo!(@pick None::<&str>), "");
    assert_eq!(concord_telemetry::repo!(@pick Some("")), "");
}

#[test]
fn repo_and_build_info_read_the_callers_build_environment() {
    let repo = option_env!("CONCORD_GIT_REPO").unwrap_or("");
    assert_eq!(concord_telemetry::repo!(), repo);
    let info = concord_telemetry::build_info!();
    assert_eq!(info.repository, repo);
    assert_eq!(info.revision, option_env!("CONCORD_GIT_SHA").unwrap_or(""));
}
