#[test]
fn derive_resolved_value() {
    let t = trybuild::TestCases::new();
    t.pass("tests/derive_resolved_value/pass-*.rs");
    t.compile_fail("tests/derive_resolved_value/fail-*.rs");
}
