use std::{collections::BTreeMap, fs, io};

use super::*;
use crate::lock::{Declaration, Snapshot};

fn read<'a>(
    files: &'a [(&'static str, &'a str)],
) -> impl FnMut(&'static str) -> io::Result<Option<Vec<u8>>> + 'a {
    |name| {
        Ok(files
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, text)| text.as_bytes().to_vec()))
    }
}

fn requests(ignored: &Ignored) -> Vec<(&str, &str)> {
    ignored
        .requests
        .iter()
        .map(|r| (r.tool.as_str(), r.version.as_str()))
        .collect()
}

fn native(ids: &[&str]) -> DeclarationMap {
    ids.iter()
        .map(|id| {
            (
                (*id).to_owned(),
                vec![Declaration {
                    file: ".nvmrc".into(),
                    field: None,
                    request: Some("24.0.0".into()),
                }],
            )
        })
        .collect()
}

#[test]
fn each_source_alone_is_reported_with_its_requests() {
    let tool_versions = discover(
        read(&[(
            ".tool-versions",
            "# pinned\nnodejs 20.11.0 # lts\npnpm 8.15.0\n\npython 3.12.1 3.11.7\n",
        )]),
        None,
    );
    assert_eq!(tool_versions.len(), 1);
    assert_eq!(tool_versions[0].manager, "asdf/mise");
    assert_eq!(
        requests(&tool_versions[0]),
        [
            ("nodejs", "20.11.0"),
            ("pnpm", "8.15.0"),
            ("python", "3.12.1 3.11.7")
        ]
    );
    for file in ["mise.toml", ".mise.toml"] {
        let mise = discover(
            read(&[(
                file,
                "[env]\nNODE_ENV = 'development'\n[tools]\nnode = \"22\"\npython = [\"3.12\", \
                 \"3.11\"]\ngo = { version = \"1.22\" }\n",
            )]),
            None,
        );
        assert_eq!((mise[0].source, mise[0].manager), (file, "mise"));
        let mut found = requests(&mise[0]);
        found.sort();
        assert_eq!(
            found,
            [("go", "1.22"), ("node", "22"), ("python", "3.12 3.11")]
        );
    }
    let volta = discover(
        read(&[]),
        Some(br#"{"name":"repo","volta":{"node":"20.11.0","pnpm":"8.15.0"}}"#),
    );
    assert_eq!(
        (volta[0].source, volta[0].manager),
        ("package.json#volta", "Volta")
    );
    assert_eq!(
        requests(&volta[0]),
        [("node", "20.11.0"), ("pnpm", "8.15.0")]
    );
}

#[test]
fn files_without_tool_requests_are_not_reported() {
    let found = discover(
        read(&[
            (".tool-versions", "# nothing pinned\n\n"),
            ("mise.toml", "[env]\nFOO = 'bar'\n"),
        ]),
        Some(br#"{"name":"repo","volta":{}}"#),
    );
    assert!(found.is_empty(), "{found:?}");
    assert!(discover(read(&[]), Some(br#"{"name":"repo"}"#)).is_empty());
}

#[test]
fn malformed_unsafe_and_unprintable_sources_are_still_reported() {
    let found = discover(
        |file| match file {
            ".tool-versions" => Ok(Some(vec![0xff, 0xfe])),
            "mise.toml" => Ok(Some(b"[tools\nnode = 22".to_vec())),
            ".mise.toml" => Err(io::Error::other("setup input exceeds byte limit")),
            _ => Ok(None),
        },
        Some(br#"{"volta":"20"}"#),
    );
    assert_eq!(
        found
            .iter()
            .map(|f| (f.source, f.unreadable))
            .collect::<Vec<_>>(),
        [
            (".tool-versions", true),
            ("mise.toml", true),
            (".mise.toml", true),
            ("package.json#volta", true)
        ]
    );
    let long = format!("nodejs {}\nruby \u{1b}[31m3\n", "1".repeat(65));
    let found = discover(read(&[(".tool-versions", &long)]), None);
    assert_eq!(
        requests(&found[0]),
        [("nodejs", "(unprintable)"), ("ruby", "(unprintable)")]
    );
    let many: String = (0..20).map(|i| format!("tool{i} 1.0.{i}\n")).collect();
    let found = discover(read(&[(".tool-versions", &many)]), None);
    assert_eq!((found[0].requests.len(), found[0].omitted), (16, 4));
    assert!(
        report(&found, &DeclarationMap::new())
            .unwrap()
            .contains("tool15 1.0.15, and 4 more")
    );
}

#[test]
fn report_marks_native_precedence_only_for_natively_declared_tools() {
    let found = discover(
        read(&[(
            ".tool-versions",
            "nodejs 20.11.0\npnpm 8.15.0\nruby 3.3.0\n",
        )]),
        Some(br#"{"volta":{"node":"18.0.0"}}"#),
    );
    assert_eq!(report(&[], &native(&["node"])), None);
    let text = report(&found, &native(&["node"])).unwrap();
    assert_eq!(
        text,
        "Ignored version manager declarations; turbo setup reads only native declarations:\n  \
         .tool-versions (asdf/mise): nodejs 20.11.0 (native node declaration takes precedence), \
         pnpm 8.15.0, ruby 3.3.0\n  package.json#volta (Volta): node 18.0.0 (native node \
         declaration takes precedence)\nDeclare versions turbo setup should manage in native \
         files such as .nvmrc, .node-version, or package.json engines, devEngines, and \
         packageManager."
    );
    let without_native = report(&found, &BTreeMap::new()).unwrap();
    assert!(
        !without_native.contains("takes precedence"),
        "{without_native}"
    );
}

#[test]
fn snapshot_reads_root_files_and_the_captured_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::write(root.join(".nvmrc"), "24.0.0\n")?;
    fs::write(root.join(".tool-versions"), "nodejs 20.11.0\n")?;
    fs::write(
        root.join("package.json"),
        r#"{"name":"repo","volta":{"node":"18.0.0"}}"#,
    )?;
    fs::create_dir(root.join("nested"))?;
    fs::write(root.join("nested/mise.toml"), "[tools]\nnode = '16'\n")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("nested/mise.toml"), root.join("mise.toml"))?;
    let snapshot = Snapshot::capture(root)?;
    let found = snapshot.ignored_declarations();
    let mut expected = vec![(".tool-versions", false)];
    if cfg!(unix) {
        // Root-only and never through a link: reported, not followed.
        expected.push(("mise.toml", true));
    }
    expected.push(("package.json#volta", false));
    assert_eq!(
        found
            .iter()
            .map(|f| (f.source, f.unreadable))
            .collect::<Vec<_>>(),
        expected
    );
    let text = report(&found, snapshot.declarations()).ok_or("missing report")?;
    assert!(text.contains("nodejs 20.11.0 (native node declaration takes precedence)"));
    assert!(!text.contains("16"), "{text}");
    Ok(())
}
