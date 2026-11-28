use super::*;

#[test]
fn sources_split_into_a_registry_and_a_package_id() {
    assert_eq!(
        registry_package("aqua:jqlang/jq"),
        Some(("aqua", "jqlang/jq"))
    );
    assert_eq!(
        registry_package("acme:acme-cli"),
        Some(("acme", "acme-cli"))
    );
    assert_eq!(
        registry_package("team_2:a.b/c_d/e-f"),
        Some(("team_2", "a.b/c_d/e-f"))
    );
    let longest = format!("aqua:{}/{}", "a".repeat(127), "b".repeat(128));
    assert!(registry_package(&longest).is_some());
    assert_eq!(registry_package(&format!("{longest}c")), None);
    let registry = "r".repeat(64);
    assert!(registry_package(&format!("{registry}:jq")).is_some());
    assert_eq!(registry_package(&format!("{registry}r:jq")), None);
    for source in [
        "jqlang/jq",
        "aqua:",
        ":jq",
        "aqua:jqlang/",
        "aqua:/jq",
        "aqua:jqlang//jq",
        "aqua:jqlang/jq@1.8.2",
        "aqua:jqlang/jq ",
        "aqua:jqlang\\jq",
        "aqua:acme:jq",
        "AQUA:jqlang/jq",
        "1aqua:jq",
        "my.registry:jq",
    ] {
        assert_eq!(registry_package(source), None, "{source}");
    }
}
