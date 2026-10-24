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
