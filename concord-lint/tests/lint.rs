use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use concord_lint::registry::{FieldVerdict, Registry};
use concord_lint::scan::scan_file;

fn scan(name: &str, source: &str) -> Vec<String> {
    let registry = Registry::shared();
    scan_file(Path::new(name), source, &registry)
        .expect("fixture parses")
        .into_iter()
        .map(|violation| violation.rule)
        .collect()
}

#[test]
fn ratchet_result_string_fires_and_result_error_does_not() {
    assert_eq!(
        scan("src/lib.rs", "fn bad() -> Result<(), String> { Ok(()) }"),
        vec!["ratchet.result_string"]
    );
    assert!(scan("src/lib.rs", "fn good() -> Result<(), MyError> { todo!() }").is_empty());
}

#[test]
fn ratchet_dropped_result_fires_and_handled_result_does_not() {
    assert_eq!(
        scan("src/lib.rs", "fn f() { let _ = work(); work().ok(); }"),
        vec!["ratchet.dropped_result", "ratchet.dropped_result"]
    );
    assert!(scan("src/lib.rs", "fn f() { let value = work(); drop(value); }").is_empty());
}

#[test]
fn ratchet_error_to_string_fires_and_error_display_does_not() {
    assert_eq!(
        scan(
            "src/lib.rs",
            "fn f() { match work() { Err(e) => e.to_string(), Ok(_) => String::new() } }"
        ),
        vec!["ratchet.error_to_string"]
    );
    assert_eq!(
        scan(
            "src/lib.rs",
            "fn f() { match work() { Err(e) => format!(\"{e}\"), Ok(_) => String::new() } }"
        ),
        vec!["ratchet.error_to_string"]
    );
    assert!(scan(
        "src/lib.rs",
        "fn f() { match work() { Err(e) => log_error(e), Ok(_) => String::new() } } "
    )
    .is_empty());
}

#[test]
fn ratchet_untraced_spawn_fires_and_instrumented_spawn_does_not() {
    assert_eq!(
        scan("src/lib.rs", "fn f() { tokio::spawn(fut); }"),
        vec!["ratchet.untraced_spawn"]
    );
    assert!(scan(
        "src/lib.rs",
        "fn f() { tokio::spawn(fut.instrument(span)); tokio::spawn(fut.in_current_span()); }"
    )
    .is_empty());
}

#[test]
fn ratchet_raw_channel_fires_and_wrapped_channel_does_not() {
    assert_eq!(
        scan("src/lib.rs", "fn f() { tokio::sync::mpsc::channel(8); }"),
        vec!["ratchet.raw_channel"]
    );
    assert!(scan("src/lib.rs", "fn f() { concord_telemetry::channel(8); }").is_empty());
}

#[test]
fn test_module_is_skipped() {
    assert!(scan(
        "src/lib.rs",
        "#[cfg(test)] mod tests { fn f() -> Result<(), String> { let _ = work(); Ok(()) } }"
    )
    .is_empty());
    assert!(scan(
        "tests/example.rs",
        "fn f() -> Result<(), String> { let _ = work(); Ok(()) }"
    )
    .is_empty());
}

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_concord-lint"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run_fixture(name: &str) -> std::process::Output {
    binary().arg(fixture(name)).output().expect("lint runs")
}

#[test]
fn tracing_messages_and_fields_are_checked() {
    let rules = scan(
        "tests/fixtures/interpolated.rs",
        "use tracing::info; fn f(id: &str) { info!(\"sent {id}\"); }",
    );
    assert_eq!(rules, vec!["message-has-values"]);

    let rules = scan(
        "tests/fixtures/unregistered.rs",
        "use tracing::info; fn f() { info!(widget = 1, \"x\"); }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);
}

#[test]
fn test_module_skips_only_the_registry_rule() {
    let source = fs::read_to_string(fixture("test_module.rs")).expect("fixture exists");
    let rules = scan("tests/fixtures/test_module.rs", &source);
    assert_eq!(rules, vec!["message-has-values", "never-log-field"]);
    assert_eq!(Registry::shared().check("stdout"), FieldVerdict::Never);
}

#[test]
fn cfg_not_test_does_not_skip_the_registry_rule() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; #[cfg(not(test))] mod m { fn f() { info!(widget = 1, \"x\"); } }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);
}

#[test]
fn cfg_all_test_and_test_fns_skip_the_registry_rule() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; #[cfg(all(test, unix))] mod m { fn f() { info!(widget = 1, \"x\"); } } #[test] fn g() { info!(widget = 2, \"x\"); }",
    );
    assert!(rules.is_empty(), "{rules:?}");
}

#[test]
fn every_rule_has_exact_fixture_output() {
    let output = run_fixture("interpolated.rs");
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(text.lines().count(), 3);
    assert!(text.lines().all(|line| line.contains("message-has-values")));

    let output = run_fixture("clean.rs");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());

    let output = run_fixture("unregistered.rs");
    let text = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("unregistered-field: widget"));

    let output = run_fixture("never.rs");
    let text = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("never-log-field: stdout"));

    let output = run_fixture("bin_main/main.rs");
    let text = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("process-exit"));

    let output = run_fixture("lib_like/other.rs");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn concord_and_shorthand_fixtures_have_expected_rules() {
    let output = run_fixture("concord.rs");
    let text = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("unregistered-field: widget"));

    let output = run_fixture("shorthand.rs");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());

    let output = run_fixture("fields.rs");
    let text = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("message-has-values"));
}

#[test]
fn local_overlay_makes_widget_clean() {
    let path = fixture("local-fields.toml");
    fs::write(
        &path,
        "[fields]\nwidget = { type = \"int\", meaning = \"test\" }\n",
    )
    .expect("write local registry");
    let output = binary()
        .args(["--local", path.to_str().expect("path is utf8")])
        .arg(fixture("unregistered.rs"))
        .output()
        .expect("lint runs");
    fs::remove_file(path).expect("remove local registry");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn baseline_tracks_new_fixed_and_duplicate_violations() {
    let dir = std::env::temp_dir().join(format!("concord-lint-baseline-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let source = dir.join("sample.rs");
    let baseline = dir.join("lint-baseline.txt");
    let three = "use tracing::info;\nfn f(x: u8) {\n    info!(\"a {x}\");\n    info!(\"b {x}\");\n    info!(\"c {x}\");\n}\n";
    fs::write(&source, three).expect("write sample");
    let baseline_arg = baseline.to_str().expect("path is utf8");

    let write = binary()
        .args(["--baseline", baseline_arg, "--write-baseline"])
        .arg(&source)
        .output()
        .expect("baseline write runs");
    assert_eq!(write.status.code(), Some(0));
    // Identical violations are counted, not collapsed: one line, count 3.
    let contents = fs::read_to_string(&baseline).expect("baseline exists");
    assert_eq!(contents.lines().count(), 1);
    assert!(contents.starts_with("3\t"), "{contents}");

    let clean = binary()
        .args(["--baseline", baseline_arg])
        .arg(&source)
        .output()
        .expect("baseline read runs");
    assert_eq!(clean.status.code(), Some(0));
    assert!(clean.stdout.is_empty());

    // A fourth copy of the same violation is new: exactly one line is listed.
    let four = three.replace("}\n", "    info!(\"d {x}\");\n}\n");
    fs::write(&source, four).expect("add a fourth violation");
    let grown = binary()
        .args(["--baseline", baseline_arg])
        .arg(&source)
        .output()
        .expect("baseline comparison runs");
    let grown_text = String::from_utf8(grown.stdout).expect("stdout is utf8");
    assert_eq!(grown.status.code(), Some(1));
    assert_eq!(grown_text.lines().count(), 1, "{grown_text}");
    assert!(grown_text.contains("message-has-values"));

    // Fixing one baselined violation exits 0 and says the baseline can shrink.
    let two = three.replace("    info!(\"c {x}\");\n", "");
    fs::write(&source, two).expect("fix one violation");
    let shrunk = binary()
        .args(["--baseline", baseline_arg])
        .arg(&source)
        .output()
        .expect("shrink comparison runs");
    assert_eq!(shrunk.status.code(), Some(0));
    assert!(shrunk.stdout.is_empty());
    assert!(String::from_utf8(shrunk.stderr)
        .expect("stderr is utf8")
        .contains("shrink"));

    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn clean_fixture_has_no_violations() {
    let output = run_fixture("clean.rs");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn equals_inside_a_string_message_is_not_a_field() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f() { info!(\"retry=3\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f() { info!(\"a == b\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn sigil_fields_are_checked_against_the_registry() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(widget: u8) { info!(%widget, \"x\"); }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(widget: u8) { info!(?widget, \"x\"); }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; struct S { id: u8 } impl S { fn f(&self) { info!(%self.id, \"x\"); } }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::warn; fn f(error: u8) { warn!(%error, \"failed\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn registry_rule_is_skipped_inside_test_fns() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; #[tokio::test] async fn t() { info!(widget = 1, \"x\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn macros_inside_unknown_macro_bodies_are_checked_once() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(x: u8) { let _ = vec![{ info!(\"bad {x}\"); 1 }]; }",
    );
    assert_eq!(rules, vec!["message-has-values"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(x: u8) -> bool { matches!(x, _ if { info!(\"bad {x}\"); true }) }",
    );
    assert_eq!(rules, vec!["message-has-values"]);
}

#[test]
fn tracing_enabled_macro_is_not_a_log_call() {
    let rules = scan(
        "src/lib.rs",
        "fn f() { let _ = tracing::enabled!(tracing::Level::INFO); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn only_tracing_macros_are_checked_by_name() {
    let rules = scan(
        "src/lib.rs",
        "use tracing_error::error; fn f(x: u8) { error!(\"bad {x}\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());

    // Imports count even when they come after the macro use.
    let rules = scan(
        "src/lib.rs",
        "fn f(x: u8) { info!(\"bad {x}\"); } use tracing::info;",
    );
    assert_eq!(rules, vec!["message-has-values"]);
}

#[test]
fn fully_qualified_event_macro_is_checked() {
    let rules = scan(
        "src/lib.rs",
        "fn f() { tracing::event!(tracing::Level::INFO, widget = 1, \"x\"); }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::Level; fn f() { tracing::event!(Level::INFO, action_id = 1, \"x\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn comparison_operator_does_not_split_fields() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(a: u8, b: u8) { info!(count = if a < b { 1 } else { 2 }, action_id = 3, \"x\"); }",
    );
    assert_eq!(rules, vec!["unregistered-field"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(a: u8, b: u8, x: u8) { info!(context = if a < b { 1 } else { 2 }, \"bad {x}\"); }",
    );
    assert_eq!(rules, vec!["message-has-values"]);
}

#[test]
fn doubled_braces_are_literal_not_interpolation() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f(y: u8) { info!(\"{{x}} {y}\"); }",
    );
    assert_eq!(rules, vec!["message-has-values"]);

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f() { info!(\"{{x}}\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn raw_string_messages_are_fixed_messages() {
    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f() { info!(r\"plain\"); }",
    );
    assert_eq!(rules, Vec::<String>::new());

    let rules = scan(
        "src/lib.rs",
        "use tracing::info; fn f() { info!(r#\"plain\"#); }",
    );
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn process_exit_is_only_flagged_in_binaries() {
    let source = "fn main() { std::process::exit(1); }";
    let rules = scan("src/bin/tool.rs", source);
    assert_eq!(rules, vec!["process-exit"]);

    let rules = scan("src/lib.rs", source);
    assert_eq!(rules, Vec::<String>::new());
}

#[test]
fn nonexistent_path_exits_with_usage_error() {
    let output = binary()
        .arg("/definitely/not/here")
        .output()
        .expect("lint runs");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn baseline_with_blank_lines_is_accepted() {
    let dir = std::env::temp_dir().join(format!("concord-lint-blank-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let source = dir.join("clean.rs");
    let baseline = dir.join("lint-baseline.txt");
    fs::write(&source, "fn f() {}\n").expect("write clean source");
    fs::write(
        &baseline,
        "1\tsrc/a.rs: message-has-values: message contains values\n\n1\tsrc/b.rs: process-exit: process::exit\n\n",
    )
    .expect("write baseline with blank lines");

    let output = binary()
        .args(["--baseline", baseline.to_str().expect("path is utf8")])
        .arg(&source)
        .output()
        .expect("baseline read runs");
    fs::remove_dir_all(&dir).expect("remove temp dir");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn events_with_only_fields_have_no_message_to_check() {
    let rules = scan(
        "src/lib.rs",
        "fn f() { tracing::event!(name: \"x\", target: \"t\", tracing::Level::INFO, action_id = 1); }",
    );
    assert!(rules.is_empty(), "{rules:?}");
    let rules = scan(
        "src/lib.rs",
        "fn f() { tracing::event!(tracing::Level::INFO, action_id = 1,); }",
    );
    assert!(rules.is_empty(), "{rules:?}");
}

#[test]
fn a_match_scrutinee_and_guard_are_scanned() {
    assert_eq!(
        scan(
            "src/lib.rs",
            "use tracing::info; fn f() { match { info!(widget = 1, \"x\"); 1 } { _ => {} } }"
        ),
        vec!["unregistered-field"]
    );
    assert_eq!(
        scan(
            "src/lib.rs",
            "use tracing::info; fn f(a: u8) { match a { 1 if { info!(widget = 1, \"x\"); true } => {} _ => {} } }"
        ),
        vec!["unregistered-field"]
    );
    assert_eq!(
        scan("src/lib.rs", "fn f() { match work().ok() { _ => {} } }"),
        Vec::<String>::new()
    );
    assert_eq!(
        scan(
            "src/lib.rs",
            "fn f() { match tokio::sync::mpsc::channel(1) { _ => {} } }"
        ),
        vec!["ratchet.raw_channel"]
    );
}

#[test]
fn a_label_recorded_through_debug_in_a_span_macro_is_reported() {
    for source in [
        "use tracing::info_span; fn f(id: u8) { let _ = info_span!(\"x\", request_id = ?id); }",
        "fn f(request_id: u8) { let _ = tracing::info_span!(\"x\", ?request_id); }",
        "fn f(id: u8) { let _ = tracing::span!(tracing::Level::INFO, \"x\", turn_id = ?id); }",
    ] {
        assert_eq!(scan("src/lib.rs", source), vec!["label-debug"], "{source}");
    }
    for source in [
        "fn f(id: u8) { let _ = tracing::info_span!(\"x\", request_id = %id); }",
        "fn f(id: u8) { let _ = tracing::info_span!(\"x\", request_id = id); }",
        "fn f(id: u8) { let _ = tracing::info_span!(\"x\", widget = ?id); }",
        "fn f(id: u8) { let _ = tracing::info_span!(\"x\", request_id = tracing::field::Empty); }",
    ] {
        assert!(scan("src/lib.rs", source).is_empty(), "{source}");
    }
}

#[test]
fn a_label_parameter_recorded_through_debug_by_instrument_is_reported() {
    // Not in TYPES_FOR_VALUE: recorded through Debug.
    assert_eq!(
        scan(
            "src/lib.rs",
            "#[tracing::instrument] fn f(request_id: Uuid) {}"
        ),
        vec!["label-debug"]
    );
    assert_eq!(
        scan(
            "src/lib.rs",
            "#[instrument] fn f(turn_id: Option<String>) {}"
        ),
        vec!["label-debug"]
    );
    // str, String, integers and references to them are recorded by value.
    for source in [
        "#[instrument] fn f(request_id: &str) {}",
        "#[instrument] fn f(request_id: String) {}",
        "#[instrument] fn f(turn_id: &u64) {}",
        "#[instrument] fn f(widget: Uuid) {}",
        // Skipped, or shadowed by fields(..).
        "#[instrument(skip(request_id))] fn f(request_id: Uuid) {}",
        "#[instrument(skip_all)] fn f(request_id: Uuid) {}",
        "#[instrument(fields(request_id = %request_id))] fn f(request_id: Uuid) {}",
        "#[instrument(skip(a), fields(turn_id = 1))] fn f(a: u8, turn_id: Uuid) {}",
        "fn f(request_id: Uuid) {}",
    ] {
        assert!(scan("src/lib.rs", source).is_empty(), "{source}");
    }
    assert_eq!(
        scan(
            "src/lib.rs",
            "#[instrument(fields(request_id = ?id))] fn f(id: Uuid) {}"
        ),
        vec!["label-debug"]
    );
    assert_eq!(
        scan(
            "src/lib.rs",
            "struct S; impl S { #[instrument] fn f(&self, action_id: Uuid) {} }"
        ),
        vec!["label-debug"]
    );
}

#[test]
fn the_reserved_label_prefix_is_not_a_field_name() {
    assert_eq!(
        scan(
            "src/lib.rs",
            "use tracing::info; fn f() { info!(\"concord.label.action_id\" = 1, \"x\"); }"
        ),
        vec!["reserved-label-prefix"]
    );
    assert_eq!(
        scan(
            "src/lib.rs",
            "fn f() { let _ = tracing::info_span!(\"x\", \"concord.label.action_id\" = 1); }"
        ),
        vec!["reserved-label-prefix"]
    );
    assert_eq!(
        Registry::shared().check("concord.label.x"),
        FieldVerdict::ReservedPrefix
    );
}
