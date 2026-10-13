use std::{collections::BTreeMap, fs};

use serde_json::json;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_setup::lock::{self, Declaration, Document, Installation, Lock, Tool};

use super::ManagedSetup;
use crate::{ColorConfig, CommandEventBuilder, FutureFlags, PruneInput, prune};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write(root: &std::path::Path, file: &str, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = root.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

fn tool(id: &str, declarations: Vec<Declaration>) -> Tool {
    Tool {
        adapter: id.into(),
        version: "24.0.0".into(),
        declarations,
        options: BTreeMap::new(),
        installation: Installation::VerifySystem {
            executables: vec![id.into()],
        },
    }
}

fn declaration(file: &str) -> Declaration {
    Declaration {
        file: file.into(),
        field: None,
        request: Some("fixture".into()),
    }
}

fn fixture(root: &std::path::Path, docker: bool) -> Result<PruneInput, Box<dyn std::error::Error>> {
    let manifest = json!({
        "name": "repo", "private": true, "workspaces": ["packages/*"],
        "packageManager": format!("npm@10.5.0+sha512.{}", "a".repeat(128)),
        "engines": {"node": "24.x"},
        "devEngines": {
            "runtime": [{"name": "node", "version": "24.x", "onFail": "error"}],
            "packageManager": {"name": "npm", "version": "10.x", "onFail": "warn"}
        }
    });
    write(root, "package.json", manifest.to_string())?;
    write(
        root,
        "packages/app/package.json",
        r#"{"name":"app","version":"1.0.0"}"#,
    )?;
    write(
        root,
        "packages/unused/package.json",
        r#"{"name":"unused","version":"1.0.0"}"#,
    )?;
    write(
        root,
        "package-lock.json",
        json!({
            "name": "repo", "lockfileVersion": 3,
            "packages": {
                "": {"name": "repo", "workspaces": ["packages/*"]},
                "packages/app": {"name": "app", "version": "1.0.0"},
                "packages/unused": {"name": "unused", "version": "1.0.0"},
                "node_modules/app": {"resolved": "packages/app", "link": true},
                "node_modules/unused": {"resolved": "packages/unused", "link": true}
            }
        })
        .to_string(),
    )?;
    write(
        root,
        "turbo.json",
        r#"{"futureFlags":{"experimentalSetup":true},"tasks":{"build":{},"unused#build":{}}}"#,
    )?;
    write(root, ".nvmrc", "24\n")?;
    write(root, ".node-version", "24.0.0\n")?;
    let tools = lock::probe_native(root)?
        .into_iter()
        .map(|(id, sources)| {
            let mut tool = tool(&id, sources);
            if id == "npm" {
                tool.version = "10.5.0".into();
            }
            (id, tool)
        })
        .collect();
    let lock = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?;
    // Noncanonical bytes prove prune preserves the committed lock, not a
    // reserialized selection (including artifact integrity in other tests).
    write(
        root,
        "turbo.lock",
        serde_json::to_vec_pretty(lock.document())?,
    )?;
    let root = AbsoluteSystemPathBuf::try_from(root)?;
    Ok(PruneInput {
        root_turbo_json_path: root.join_component("turbo.json"),
        repo_root: root,
        color_config: ColorConfig::new(true),
        scope: vec!["app".into()],
        docker,
        production: false,
        output_dir: "out".into(),
        use_gitignore: true,
        allow_missing_package_manager: false,
        future_flags: FutureFlags {
            experimental_setup: true,
            ..Default::default()
        },
    })
}

#[tokio::test]
async fn locked_js_sources_survive_pruned_manifests_in_all_layouts() -> TestResult {
    for docker in [false, true] {
        let temp = tempfile::tempdir()?;
        let input = fixture(temp.path(), docker)?;
        write(temp.path(), ".gitignore", ".nvmrc\n.node-version\n")?;
        let before = fs::read(temp.path().join("turbo.lock"))?;
        let sources = lock::probe_native(temp.path())?;
        prune(input, CommandEventBuilder::new("prune")).await?;
        for layout in if docker {
            vec!["out/full", "out/json"]
        } else {
            vec!["out"]
        } {
            let output = temp.path().join(layout);
            assert_eq!(fs::read(output.join("turbo.lock"))?, before);
            assert_eq!(lock::probe_native(&output)?, sources);
            let locked = Lock::read(&output)?.ok_or("missing output lock")?;
            assert!(locked.matches_native(&lock::probe_native(&output)?)?);
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(output.join("package.json"))?)?;
            assert_eq!(manifest["workspaces"], json!(["packages/app"]));
            let config: serde_json::Value =
                serde_json::from_slice(&fs::read(output.join("turbo.json"))?)?;
            assert_eq!(config["futureFlags"]["experimentalSetup"], true);
            assert!(config["tasks"].get("unused#build").is_none());
            assert!(!output.join("packages/unused").exists());
            assert_eq!(fs::read(output.join(".nvmrc"))?, b"24\n");
            assert_eq!(fs::read(output.join(".node-version"))?, b"24.0.0\n");
        }
    }
    Ok(())
}

#[tokio::test]
async fn docker_install_layer_retains_jsonc_setup_opt_in() -> TestResult {
    let temp = tempfile::tempdir()?;
    let mut input = fixture(temp.path(), true)?;
    fs::rename(
        temp.path().join("turbo.json"),
        temp.path().join("turbo.jsonc"),
    )?;
    input.root_turbo_json_path = input.repo_root.join_component("turbo.jsonc");
    prune(input, CommandEventBuilder::new("prune")).await?;
    for layout in ["out/full", "out/json"] {
        let output = temp.path().join(layout);
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("turbo.jsonc"))?)?;
        assert_eq!(config["futureFlags"]["experimentalSetup"], true);
        assert!(config["tasks"].get("unused#build").is_none());
        assert!(!output.join("turbo.json").exists());
    }
    Ok(())
}

#[tokio::test]
async fn native_and_additional_provenance_is_copied_without_adapter_allowlists() -> TestResult {
    for docker in [false, true] {
        let temp = tempfile::tempdir()?;
        let input = fixture(temp.path(), docker)?;
        let mut document = Lock::read(temp.path())?
            .ok_or("missing fixture lock")?
            .document()
            .clone();
        let files = [
            (
                "rust",
                "rust-toolchain.toml",
                "[toolchain]\nchannel = \"stable\"\n",
            ),
            ("python", ".python-version", "3.14\n"),
            ("go", "go.work", "go 1.25\n"),
            (
                "custom",
                "config/tool-versions.toml",
                "version = \"1.0.0\"\n",
            ),
        ];
        for (id, file, contents) in files {
            write(temp.path(), file, contents)?;
            document
                .tools
                .insert(id.into(), tool(id, vec![declaration(file)]));
        }
        // A real managed artifact also remains byte-identical: no resolution or
        // integrity metadata is synthesized by prune.
        document
            .tools
            .get_mut("custom")
            .ok_or("missing tool")?
            .installation = Installation::Managed {
            artifacts: BTreeMap::from([(
                lock::Platform::Any,
                BTreeMap::from([(
                    "main".into(),
                    lock::Artifact {
                        url: "https://example.com/tool.tar.gz".into(),
                        sha256: "a".repeat(64),
                        format: lock::Format::TarGz,
                        root_prefix: Some("tool".into()),
                        destination: None,
                        executables: BTreeMap::from([("custom".into(), "bin/custom".into())]),
                    },
                )]),
            )]),
        };
        let bytes = Lock::new(document)?.canonical_bytes()?;
        write(temp.path(), "turbo.lock", &bytes)?;
        prune(input, CommandEventBuilder::new("prune")).await?;
        for layout in if docker {
            vec!["out/full", "out/json"]
        } else {
            vec!["out"]
        } {
            let output = temp.path().join(layout);
            assert_eq!(fs::read(output.join("turbo.lock"))?, bytes);
            for (_, file, contents) in files {
                assert_eq!(fs::read(output.join(file))?, contents.as_bytes());
            }
            assert!(Lock::read(&output)?.is_some());
        }
    }
    Ok(())
}

#[tokio::test]
async fn unconfigured_prune_ignores_locks_and_undeclared_version_files() -> TestResult {
    for (flag, lock_exists) in [(false, true), (true, false)] {
        let temp = tempfile::tempdir()?;
        let mut input = fixture(temp.path(), true)?;
        input.future_flags.experimental_setup = flag;
        if lock_exists {
            write(temp.path(), "turbo.lock", "not a lock")?;
        } else {
            fs::remove_file(temp.path().join("turbo.lock"))?;
        }
        prune(input, CommandEventBuilder::new("prune")).await?;
        for layout in ["out/full", "out/json"] {
            let output = temp.path().join(layout);
            assert!(!output.join("turbo.lock").exists());
            assert!(!output.join(".nvmrc").exists());
            assert!(!output.join(".node-version").exists());
            if layout == "out/json" {
                assert!(!output.join("turbo.json").exists());
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn unsafe_missing_and_malformed_sources_fail_before_output() -> TestResult {
    for file in ["missing.toml", "../outside", "/absolute", "a\\b", "config"] {
        let temp = tempfile::tempdir()?;
        let input = fixture(temp.path(), true)?;
        let mut document = Lock::read(temp.path())?
            .ok_or("missing fixture lock")?
            .document()
            .clone();
        document
            .tools
            .insert("custom".into(), tool("custom", vec![declaration(file)]));
        if file == "config" {
            fs::create_dir(temp.path().join("config"))?;
        }
        write(temp.path(), "turbo.lock", serde_json::to_vec(&document)?)?;
        assert!(
            prune(input, CommandEventBuilder::new("prune"))
                .await
                .is_err(),
            "{file}"
        );
        assert!(!temp.path().join("out").exists());
    }
    let temp = tempfile::tempdir()?;
    let input = fixture(temp.path(), true)?;
    for bytes in [
        b"{}".as_slice(),
        b"{\"schemaVersion\":99,\"tools\":{}}",
        b"{\"schemaVersion\":0,\"schemaVersion\":0,\"tools\":{}}",
    ] {
        write(temp.path(), "turbo.lock", bytes)?;
        assert!(ManagedSetup::plan(&input).is_err());
        assert!(!temp.path().join("out").exists());
    }
    write(
        temp.path(),
        "turbo.lock",
        vec![b' '; lock::MAX_LOCK_BYTES + 1],
    )?;
    assert!(ManagedSetup::plan(&input).is_err());
    assert!(!temp.path().join("out").exists());
    Ok(())
}

#[tokio::test]
async fn destination_collisions_are_rejected_before_any_copy() -> TestResult {
    let temp = tempfile::tempdir()?;
    let input = fixture(temp.path(), true)?;
    fs::create_dir_all(temp.path().join("out/full/.nvmrc"))?;
    fs::create_dir_all(temp.path().join("out/json"))?;
    assert!(
        prune(input, CommandEventBuilder::new("prune"))
            .await
            .is_err()
    );
    assert!(!temp.path().join("out/full/turbo.lock").exists());
    assert!(fs::read_dir(temp.path().join("out/json"))?.next().is_none());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn non_regular_sources_are_rejected_without_opening_them() -> TestResult {
    let temp = tempfile::tempdir()?;
    let input = fixture(temp.path(), false)?;
    fs::remove_file(temp.path().join(".nvmrc"))?;
    let _socket = std::os::unix::net::UnixListener::bind(temp.path().join(".nvmrc"))?;
    assert!(
        prune(input, CommandEventBuilder::new("prune"))
            .await
            .is_err()
    );
    assert!(!temp.path().join("out").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn outside_repo_symlink_parents_are_rejected_without_writes() -> TestResult {
    use std::os::unix::fs::symlink;
    for docker in [false, true] {
        for absolute in [false, true] {
            let temp = tempfile::tempdir()?;
            let repo = temp.path().join("repo");
            let destination = temp.path().join("destination");
            fs::create_dir(&destination)?;
            write(&destination, "sentinel", "untouched")?;
            let link = temp.path().join("output-link");
            symlink(&destination, &link)?;
            let mut input = fixture(&repo, docker)?;
            input.output_dir = if absolute {
                link.join("nested").to_string_lossy().into_owned()
            } else {
                "../output-link/nested".into()
            };
            let result = prune(input, CommandEventBuilder::new("prune")).await;
            assert!(result.is_err(), "docker={docker}, absolute={absolute}");
            assert_eq!(fs::read(destination.join("sentinel"))?, b"untouched");
            assert_eq!(fs::read_dir(&destination)?.count(), 1);
            assert!(!destination.join("nested").exists());
            assert!(!repo.join("out").exists());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn macos_tmp_alias_remains_a_valid_output_parent() -> TestResult {
    for docker in [false, true] {
        let temp = tempfile::tempdir_in("/tmp")?;
        let mut input = fixture(&temp.path().join("repo"), docker)?;
        input.output_dir = temp.path().join("output").to_string_lossy().into_owned();
        prune(input, CommandEventBuilder::new("prune")).await?;
        for layout in if docker {
            vec!["output/full", "output/json"]
        } else {
            vec!["output"]
        } {
            assert!(Lock::read(&temp.path().join(layout))?.is_some());
        }
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_sources_and_destinations_are_rejected_before_output() -> TestResult {
    use std::os::unix::fs::symlink;
    for file in ["turbo.lock", ".nvmrc", "turbo.json", "config/tool.toml"] {
        let temp = tempfile::tempdir()?;
        let input = fixture(temp.path(), true)?;
        if file.starts_with("config/") {
            let mut document = Lock::read(temp.path())?
                .ok_or("missing fixture lock")?
                .document()
                .clone();
            document
                .tools
                .insert("custom".into(), tool("custom", vec![declaration(file)]));
            write(
                temp.path(),
                "turbo.lock",
                Lock::new(document)?.canonical_bytes()?,
            )?;
            fs::create_dir(temp.path().join("real-config"))?;
            write(temp.path(), "real-config/tool.toml", "version")?;
            symlink("real-config", temp.path().join("config"))?;
        } else {
            fs::rename(temp.path().join(file), temp.path().join("real-source"))?;
            symlink("real-source", temp.path().join(file))?;
        }
        assert!(
            prune(input, CommandEventBuilder::new("prune"))
                .await
                .is_err()
        );
        assert!(!temp.path().join("out").exists());
    }
    let temp = tempfile::tempdir()?;
    let input = fixture(temp.path(), true)?;
    let external = tempfile::tempdir()?;
    fs::create_dir_all(temp.path().join("out/json"))?;
    symlink(external.path(), temp.path().join("out/full"))?;
    assert!(ManagedSetup::plan(&input).is_err());
    assert!(fs::read_dir(external.path())?.next().is_none());
    assert!(fs::read_dir(temp.path().join("out/json"))?.next().is_none());
    // The final directory need not exist for a parent link to be unsafe.
    let mut nested = input;
    symlink(external.path(), temp.path().join("output-link"))?;
    nested.output_dir = "output-link/nested".into();
    assert!(ManagedSetup::plan(&nested).is_err());
    assert!(!external.path().join("nested").exists());
    Ok(())
}
