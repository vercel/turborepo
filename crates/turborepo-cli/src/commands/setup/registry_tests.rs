use super::*;

fn configure(f: &Fixture, registries: Value, tools: Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"registries": registries, "additionalTools": tools}});
    fs::write(f.owned.root().join("turbo.json"), config.to_string()).unwrap();
}

#[test]
fn removing_a_registry_is_drift_when_frozen_and_unresolved_locally() {
    let (f, u) = fixture();
    configure(&f, json!(["aqua"]), json!({"fixture": "1.2.3"}));
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    let (bytes, hits) = (lock(&f), u.hits());

    configure(&f, json!([]), json!({"fixture": "1.2.3"}));
    for flags in [FROZEN, &["--tools-only", "--no-frozen", "--force"]] {
        let error = run(&f, &u, flags).unwrap_err();
        assert!(
            matches!(&error, Error::AdditionalToolDrift(ids)
                if ids.iter().map(String::as_str).collect::<Vec<_>>() == ["fixture"]),
            "{flags:?}: {error}"
        );
    }
    let error = run(&f, &u, LOCAL).unwrap_err();
    let message = error.to_string();
    assert!(
        matches!(
            &error,
            Error::Catalog(turborepo_setup_catalog::Error::UnknownName { .. })
        ) && message.contains("setup.registries is empty"),
        "{message}"
    );
    assert_eq!(u.hits(), hits);
    assert_eq!(lock(&f), bytes);

    configure(&f, json!(["aqua"]), json!({"fixture": "1.2.3"}));
    assert_eq!(run(&f, &u, FROZEN).unwrap(), 0);
    assert_eq!(lock(&f), bytes);
}
