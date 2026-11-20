use std::collections::BTreeMap;

use semver::Version;

use super::*;

fn discover(files: &[(&'static str, &str)]) -> Result<Option<UvRequirement>, UvDiscoveryError> {
    let files: BTreeMap<_, _> = files.iter().copied().collect();
    UvRequirement::read_with(|file, _| Ok(files.get(file).map(|text| text.as_bytes().to_vec())))
}

#[test]
fn uv_toml_replaces_pyproject_settings_like_uv() {
    let pyproject = "[tool.uv]\nrequired-version = \">=0.5\"\n";
    let found = discover(&[
        ("uv.toml", "required-version = \"0.6.1\"\n"),
        ("pyproject.toml", pyproject),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        found.source(),
        &UvSource {
            file: "uv.toml",
            field: "required-version",
            request: "0.6.1".into()
        }
    );
    assert!(found.matches(&Version::new(0, 6, 1)));
    assert!(!found.matches(&Version::new(0, 6, 2)));
    // A uv.toml without a request still shadows [tool.uv].
    assert!(
        discover(&[
            ("uv.toml", "native-tls = true\n"),
            ("pyproject.toml", pyproject)
        ])
        .unwrap()
        .is_none()
    );
    let found = discover(&[("pyproject.toml", pyproject)]).unwrap().unwrap();
    assert_eq!(
        found.declaration(),
        Declaration {
            file: "pyproject.toml".into(),
            field: Some("tool.uv.required-version".into()),
            request: Some(">=0.5".into()),
        }
    );
    assert!(found.matches(&Version::new(0, 13, 0)));
    assert!(!found.matches(&Version::parse("0.13.0-rc.1").unwrap()));
}

#[test]
fn absent_declarations_are_not_floating_requests() {
    for files in [
        vec![],
        vec![("pyproject.toml", "[project]\nname = \"app\"\n")],
        vec![("pyproject.toml", "[tool.ruff]\nline-length = 100\n")],
        vec![("pyproject.toml", "[tool.uv]\ndev-dependencies = []\n")],
    ] {
        assert!(discover(&files).unwrap().is_none());
    }
}

#[test]
fn invalid_sources_identify_the_file_without_echoing_contents() {
    for (file, text) in [
        ("pyproject.toml", "[tool.uv]\nrequired-version = 5\n"),
        ("pyproject.toml", "tool = 1\n"),
        ("pyproject.toml", "[tool]\nuv = \"secret\"\n"),
        (
            "uv.toml",
            "required-version = \"0.5\"\nrequired-version = \"0.6\"\n",
        ),
        ("uv.toml", "token = \"secret\"\n[broken"),
    ] {
        let error = discover(&[(file, text)]).unwrap_err();
        assert!(
            matches!(&error, UvDiscoveryError::Invalid { file: found, .. } if *found == file),
            "{text}"
        );
        assert!(!error.to_string().contains("secret"));
    }
    assert!(matches!(
        discover(&[("uv.toml", "required-version = \"===0.5\"\n")]),
        Err(UvDiscoveryError::Specifier {
            file: "uv.toml",
            ..
        })
    ));
    let large = "#".repeat(MAX_CONFIG_BYTES + 1);
    assert!(matches!(
        discover(&[("pyproject.toml", &large)]),
        Err(UvDiscoveryError::TooLarge("pyproject.toml"))
    ));
    assert!(matches!(
        UvRequirement::read_with(|_, _| Err(io::Error::other("denied"))),
        Err(UvDiscoveryError::Read("uv.toml", _))
    ));
}
