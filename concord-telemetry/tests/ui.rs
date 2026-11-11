#[test]
fn derive_diagnostics() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
    // `concord_telemetry_proto::v1::Principal` is the type the derive accepts.
    tests.pass("tests/ui_pass/*.rs");
}
