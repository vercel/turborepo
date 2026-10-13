use super::*;

#[test]
fn mismatched_native_adapter_and_presence_are_not_false_absence() {
    let mut document = fixture("24.0.0").document().clone();
    document.tools.get_mut("node").unwrap().adapter = "generic-download".into();
    assert!(
        !Lock::new(document)
            .unwrap()
            .matches_native(&BTreeMap::new())
            .unwrap()
    );
    let current = BTreeMap::from([(
        "node".into(),
        vec![Declaration {
            file: "package.json".into(),
            field: Some("devEngines.runtime".into()),
            request: None,
        }],
    )]);
    let lock = selection(current.clone(), "24.0.0");
    let mut missing = current.clone();
    missing.get_mut("node").unwrap()[0].field = None;
    assert!(!lock.matches_native(&missing).unwrap());
    missing = current.clone();
    missing.get_mut("node").unwrap()[0].request = Some("*".into());
    assert!(!lock.matches_native(&missing).unwrap());
    missing.get_mut("node").unwrap().clear();
    assert!(lock.matches_native(&missing).is_err());
}

#[test]
fn canonical_output_byte_limit_fails_before_any_write() {
    let mut document = fixture("24.0.0").document().clone();
    let template = document.tools.remove("node").unwrap();
    for index in 0..4 {
        let mut tool = template.clone();
        tool.adapter = "generic-download".into();
        tool.installation = Installation::VerifySystem {
            executables: vec![format!("tool-{index}")],
        };
        tool.declarations = (0..64)
            .map(|source| Declaration {
                file: format!("source-{source}"),
                field: None,
                request: Some("x".repeat(3900)),
            })
            .collect();
        document.tools.insert(format!("tool-{index}"), tool);
    }
    let mut remaining = MAX_LOCK_BYTES - serde_json::to_vec(&document).unwrap().len();
    for tool in document.tools.values_mut() {
        for declaration in &mut tool.declarations {
            let request = declaration.request.as_mut().unwrap();
            let added = remaining.min(4096 - request.len());
            request.extend(std::iter::repeat_n('x', added));
            remaining -= added;
        }
    }
    assert_eq!(remaining, 0);
    let lock = Lock::new(document).unwrap();
    assert_eq!(lock.canonical_bytes(), Err(Error::TooLarge));
    let root = tempfile::tempdir().unwrap();
    assert!(write(root.path(), &lock).is_err());
    assert!(files(root.path()).is_empty());
    fs::write(
        root.path().join(LOCK_NAME),
        fixture("24.0.0").canonical_bytes().unwrap(),
    )
    .unwrap();
    let before = fs::read(root.path().join(LOCK_NAME)).unwrap();
    assert!(write(root.path(), &lock).is_err());
    assert_eq!(fs::read(root.path().join(LOCK_NAME)).unwrap(), before);
    assert_eq!(files(root.path()), vec![LOCK_NAME]);
}
