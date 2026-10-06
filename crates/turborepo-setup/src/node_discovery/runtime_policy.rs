use serde_json::json;

use super::*;

#[test]
fn authored_policy_presence_is_visible_to_source_comparison() -> TestResult {
    for runtime in [
        json!({"name":"node"}),
        json!({"name":"node","version":"24.x"}),
    ] {
        for array in [false, true] {
            let root = root(&[])?;
            let mut manifest = json!({"devEngines":{"runtime":if array {json!([runtime.clone()])} else {runtime.clone()}}});
            root.put("package.json", manifest.to_string())?;
            let before: Vec<_> = root.read()?.sources().cloned().collect();
            let entry = if array {
                &mut manifest["devEngines"]["runtime"][0]
            } else {
                &mut manifest["devEngines"]["runtime"]
            };
            entry["onFail"] = json!("error");
            root.put("package.json", manifest.to_string())?;
            let discovered = root.read()?;
            let after: Vec<_> = discovered.sources().cloned().collect();
            assert_ne!(before, after);
            assert_eq!(after.len(), before.len() + 1);
            let policy = after.last().ok_or("missing policy source")?;
            assert_eq!(policy.file, "package.json");
            assert_eq!(
                policy.field.as_deref(),
                Some(if array {
                    "devEngines.runtime[0].onFail"
                } else {
                    "devEngines.runtime.onFail"
                })
            );
            assert_eq!(policy.request.as_deref(), Some("error"));
            assert_eq!(discovered.resolve(&catalog()?)?.sources, after);
            let entry = if array {
                &mut manifest["devEngines"]["runtime"][0]
            } else {
                &mut manifest["devEngines"]["runtime"]
            };
            entry
                .as_object_mut()
                .ok_or("missing runtime")?
                .remove("onFail");
            root.put("package.json", manifest.to_string())?;
            assert_eq!(root.read()?.sources().cloned().collect::<Vec<_>>(), before);
        }
    }
    Ok(())
}

#[test]
fn policy_mutations_are_diagnosed_not_silently_ignored() -> TestResult {
    for value in [
        json!("warn"),
        json!("ignore"),
        json!(false),
        json!(null),
        json!(12),
        json!({}),
        json!(["error"]),
        json!("credential-secret"),
    ] {
        let root = root(&[(".nvmrc", "24.x")])?;
        root.put(
            "package.json",
            json!({"devEngines":{"runtime":[{"name":"node","version":"24.x","onFail":value}]}})
                .to_string(),
        )?;
        let error = root.error()?;
        assert!(error.contains("package.json#devEngines.runtime[0].onFail"));
        assert!(!error.contains("credential-secret"));
        assert!(root.resolve().is_err()); // No fallback to the valid .nvmrc.
    }
    Ok(())
}

#[test]
fn policy_provenance_is_not_an_unconstrained_runtime_alternative() -> TestResult {
    let root = root(&[(
        "package.json",
        r#"{"devEngines":{"runtime":[{"name":"node","version":"22.x","onFail":"error"},{"name":"node","version":"24.x","onFail":"error"}]}}"#,
    )])?;
    let requirements = root.read()?;
    let resolved = requirements.resolve(&catalog()?)?;
    assert_eq!(resolved.version.to_string(), "24.10.0"); // 25.1.0 is not admitted by policy metadata.
    assert_eq!(resolved.sources.len(), 4);
    assert_eq!(
        resolved.selection_source.field.as_deref(),
        Some("devEngines.runtime[1].version")
    );
    let no_match = [NodeRelease::new("25.1.0", None)?];
    assert!(matches!(
        requirements.resolve(&no_match),
        Err(NodeDiscoveryError::NoMatchingRelease { .. })
    ));
    Ok(())
}
