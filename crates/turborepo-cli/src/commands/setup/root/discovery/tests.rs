#![allow(clippy::unwrap_used)]

use std::{ffi::OsString, fs};

use super::*;
use crate::{cli::Command, commands::setup};

const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;

// Pure parser/discovery fixtures work on every platform. They do not provision
// artifacts or imply host support. The locked CLI owns supported-host success
// gates (macOS/GNU Linux x64/arm64) and unsupported Unix/musl fail-closed
// tests.
struct Fixture {
    _temp: tempfile::TempDir,
    root: AbsoluteSystemPathBuf,
    nested: AbsoluteSystemPathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        // Root inference recognizes directory OR file Git boundaries without
        // executing Git. Isolate all ancestor searches from the host checkout.
        root.join_component(".git").create_dir_all().unwrap();
        root.join_component("turbo.json")
            .create_with_contents(ENABLED)
            .unwrap();
        let nested = root.join_components(&["apps", "web"]);
        nested.create_dir_all().unwrap();
        Self {
            _temp: temp,
            root,
            nested,
        }
    }

    fn args(&self, cwd: &str, config: Option<&str>) -> Args {
        let mut words = vec!["turbo", "setup", "--cwd", cwd, "--frozen", "--tools-only"];
        if let Some(config) = config {
            words.extend(["--root-turbo-json", config]);
        }
        Args::parse_args(words.into_iter().map(OsString::from).collect()).unwrap()
    }

    fn capture(&self, cwd: &str, config: Option<&str>) -> Discovery {
        Discovery::capture_at(&self.args(cwd, config), &self.root).unwrap()
    }

    fn discovery(&self) -> Discovery {
        self.capture("apps/web", None)
    }
}

#[test]
fn real_parser_captures_relative_and_absolute_invocation_once() {
    let f = Fixture::new();
    for cwd in ["apps/web", f.nested.as_str()] {
        let d = f.capture(cwd, None);
        assert!(d.explicit_cwd);
        assert_eq!(d.cwd, f.nested);
        assert_eq!(d.root_path(), &*f.root);
        assert_eq!(d.snapshot_root().unwrap(), &*f.root);
        assert!(d.flags().experimental_setup);
        assert!(d.source_eligibility().node());
        assert!(!d.source_eligibility().cargo());
        assert!(!d.source_eligibility().python());
        assert!(!d.source_eligibility().go());
        assert!(d.revalidate().unwrap());
    }
    let args = Args::parse_args(vec!["turbo".into(), "setup".into()]).unwrap();
    let d = Discovery::capture_at(&args, &f.nested).unwrap();
    assert!(!d.explicit_cwd);
    assert_eq!(d.root_path(), &*f.root);
    assert!(d.revalidate().unwrap());
}

#[test]
fn explicit_config_is_invocation_relative_not_cwd_relative() {
    let f = Fixture::new();
    f.root
        .join_component("turbo.jsonc")
        .create_with_contents(ENABLED)
        .unwrap();
    // Without an explicit choice, two config files conflict as before.
    assert!(matches!(
        Discovery::capture_at(&f.args("apps/web", None), &f.root),
        Err(Error::Config(_))
    ));
    for config in ["turbo.jsonc", f.root.join_component("turbo.jsonc").as_str()] {
        let d = f.capture("apps/web", Some(config));
        assert_eq!(
            d.config.as_ref().unwrap(),
            &f.root.join_component("turbo.jsonc")
        );
        assert_eq!(d.snapshot_root().unwrap(), &*f.root);
        assert!(d.revalidate().unwrap());
    }
}

#[test]
fn unsupported_custom_config_still_gates_but_cannot_be_guarded() {
    let f = Fixture::new();
    f.root
        .join_component("custom.json")
        .create_with_contents(ENABLED)
        .unwrap();
    let d = f.capture("apps/web", Some("custom.json"));
    assert!(d.flags().experimental_setup);
    assert!(matches!(d.snapshot_root(), Err(Error::UnsupportedConfig)));
    assert!(matches!(d.revalidate(), Err(Error::UnsupportedConfig)));
    assert_eq!(format!("{d:?}"), "Discovery { .. }");
    assert!(
        !Error::UnsupportedConfig
            .to_string()
            .contains(f.root.as_str())
    );
    assert!(!f.root.join_component(".turbo").exists());
}

#[test]
fn same_root_flag_and_eligibility_drift_is_not_a_root_only_check() {
    for flag in [
        "experimentalSetup",
        "experimentalCargoWorkspaces",
        "experimentalPythonWorkspaces",
        "experimentalGoWorkspaces",
        "errorsOnlyShowHash",
    ] {
        let f = Fixture::new();
        let d = f.discovery();
        let changed = if flag == "experimentalSetup" {
            r#"{"futureFlags":{"experimentalSetup":false}}"#.to_owned()
        } else {
            format!(r#"{{"futureFlags":{{"experimentalSetup":true,"{flag}":true}}}}"#)
        };
        f.root
            .join_component("turbo.json")
            .create_with_contents(&changed)
            .unwrap();
        assert!(!d.revalidate().unwrap(), "{flag}");
        assert_eq!(infer(&f.nested, true, None).unwrap().path, f.root);
        // Expected state is immutable: don't adopt flags from the second read.
        assert_eq!(
            d.flags(),
            FutureFlags {
                experimental_setup: true,
                ..Default::default()
            }
        );
        assert!(!d.source_eligibility().cargo());
    }
}

#[test]
fn non_js_markers_and_version_files_are_ignored_until_their_flag_is_on() {
    for (flag, marker, body, version) in [
        (
            "experimentalCargoWorkspaces",
            "Cargo.toml",
            "[workspace]\nmembers = []\n",
            "rust-toolchain.toml",
        ),
        (
            "experimentalPythonWorkspaces",
            "pyproject.toml",
            "[tool.uv.workspace]\nmembers = []\n",
            ".python-version",
        ),
        ("experimentalGoWorkspaces", "go.work", "go 1.22\n", "go.mod"),
    ] {
        let f = Fixture::new();
        let d = f.discovery();
        f.nested
            .join_component(marker)
            .create_with_contents(body)
            .unwrap();
        f.nested
            .join_component(version)
            .create_with_contents("ignored version request")
            .unwrap();
        assert!(d.revalidate().unwrap(), "inactive {marker}");
        f.root
            .join_component("turbo.json")
            .create_with_contents(format!(
                r#"{{"futureFlags":{{"experimentalSetup":true,"{flag}":true}}}}"#
            ))
            .unwrap();
        assert!(
            matches!(d.revalidate(), Err(Error::Nested { .. })),
            "active {marker}"
        );
        fs::remove_file(f.nested.join_component(marker)).unwrap();
        let active = f.discovery();
        let eligible = active.source_eligibility();
        assert_eq!(eligible.cargo(), flag == "experimentalCargoWorkspaces");
        assert_eq!(eligible.python(), flag == "experimentalPythonWorkspaces");
        assert_eq!(eligible.go(), flag == "experimentalGoWorkspaces");
        assert!(active.revalidate().unwrap());
    }
}

#[test]
fn new_nested_js_root_markers_preserve_root_diagnostics() {
    for (marker, body) in [
        ("turbo.json", ENABLED),
        ("turbo.jsonc", ENABLED),
        ("pnpm-workspace.yaml", "packages: []\n"),
        ("package.json", r#"{"workspaces":[]}"#),
        ("package.json", r#"{"name":"independent"}"#),
    ] {
        for explicit_config in [None, Some("turbo.json")] {
            let f = Fixture::new();
            let d = f.capture("apps/web", explicit_config);
            f.nested
                .join_component(marker)
                .create_with_contents(body)
                .unwrap();
            if marker.starts_with("turbo.json") && explicit_config.is_none() {
                // Exact --cwd becomes authoritative once it has its own root
                // config. Identity changes, rather than becoming ambiguous.
                assert!(!d.revalidate().unwrap(), "{marker}");
            } else {
                assert!(
                    matches!(d.revalidate(), Err(Error::Nested { .. })),
                    "{marker}"
                );
            }
            assert!(!f.root.join_component(".turbo").exists());
        }
    }
}

#[test]
fn new_ancestor_config_is_ambiguous_and_disabled_setup_has_no_eligible_sources() {
    let f = Fixture::new();
    let start = f.nested.join_component("src");
    start.create_dir_all().unwrap();
    let d = f.capture(start.as_str(), None);
    f.nested
        .join_component("turbo.json")
        .create_with_contents(ENABLED)
        .unwrap();
    assert!(matches!(d.revalidate(), Err(Error::Ambiguous { .. })));
    f.root.join_component("turbo.json").create_with_contents(
        r#"{"futureFlags":{"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true}}"#,
    ).unwrap();
    let disabled = f.capture(".", None);
    let policy = disabled.source_eligibility();
    assert!(!policy.node() && !policy.cargo() && !policy.python() && !policy.go());
    assert!(disabled.revalidate().unwrap());
}

#[test]
fn included_js_member_stays_valid_until_outer_workspace_membership_changes() {
    let f = Fixture::new();
    f.root
        .join_component("package.json")
        .create_with_contents(r#"{"workspaces":["apps/*"]}"#)
        .unwrap();
    f.nested
        .join_component("package.json")
        .create_with_contents(r#"{"name":"web"}"#)
        .unwrap();
    f.nested
        .join_component("turbo.json")
        .create_with_contents(r#"{"extends":["//"]}"#)
        .unwrap();
    let d = f.discovery();
    assert!(d.revalidate().unwrap());
    f.root
        .join_component("package.json")
        .create_with_contents(r#"{"workspaces":["other/*"]}"#)
        .unwrap();
    assert!(matches!(d.revalidate(), Err(Error::Nested { .. })));
}

#[test]
fn config_selection_drift_and_malformed_roots_are_not_hidden() {
    let f = Fixture::new();
    let d = f.discovery();
    fs::rename(
        f.root.join_component("turbo.json"),
        f.root.join_component("turbo.jsonc"),
    )
    .unwrap();
    assert!(!d.revalidate().unwrap());
    f.root
        .join_component("turbo.jsonc")
        .create_with_contents("{invalid")
        .unwrap();
    assert!(matches!(d.revalidate(), Err(Error::TurboJson(_))));
}

#[test]
fn explicit_exact_root_does_not_begin_searching_ancestors_on_recheck() {
    let f = Fixture::new();
    f.nested
        .join_component("turbo.json")
        .create_with_contents(ENABLED)
        .unwrap();
    let exact = f.capture("apps/web", None);
    assert_eq!(exact.root_path(), &*f.nested);
    assert!(exact.revalidate().unwrap());
    let args = Args::parse_args(vec!["turbo".into(), "setup".into()]).unwrap();
    assert!(matches!(
        Discovery::capture_at(&args, &f.nested),
        Err(Error::Ambiguous { .. })
    ));
}

#[test]
fn modeled_download_checks_fail_before_storage_and_preserve_prior_selection() {
    // Context precursor only: these closures model the two future call sites;
    // this is NOT qualification of the blocked executor or Store transaction.
    for prior in [false, true] {
        let f = Fixture::new();
        let d = f.discovery();
        let config_before = fs::read(f.root.join_component("turbo.json")).unwrap();
        let inventory = f
            .root
            .join_components(&[".turbo", "tools", "manifest.json"]);
        if prior {
            inventory.parent().unwrap().create_dir_all().unwrap();
            inventory.create_with_contents("prior selection").unwrap();
        }
        let download = || {
            // Simulate an owned fixture's mid-download mutation. Adding both a
            // Git boundary and config makes normal inference select a deeper
            // root instead of simply producing an ambiguous-root diagnostic.
            f.nested.join_component(".git").create_dir_all().unwrap();
            f.nested
                .join_component("turbo.json")
                .create_with_contents(ENABLED)
                .unwrap();
        };
        download();
        assert_eq!(infer(&f.nested, true, None).unwrap().path, f.nested);
        assert_eq!(
            config_before,
            fs::read(f.root.join_component("turbo.json")).unwrap()
        );
        assert!(!d.revalidate().unwrap());
        let guarded_publish = || -> Result<bool, Error> {
            if !d.revalidate()? {
                return Ok(false);
            }
            inventory.parent().unwrap().create_dir_all().unwrap();
            inventory.create_with_contents("new selection").unwrap();
            Ok(true)
        };
        assert!(!guarded_publish().unwrap());
        if prior {
            assert_eq!(fs::read_to_string(inventory).unwrap(), "prior selection");
        } else {
            assert!(!f.root.join_component(".turbo").exists());
        }
    }
}

#[test]
fn production_run_preserves_disabled_parse_normalization_and_not_implemented() {
    let f = Fixture::new();
    let dispatch = |flags: &[&str]| {
        let words = ["turbo", "setup", "--cwd", f.root.as_str()]
            .into_iter()
            .chain(flags.iter().copied());
        let args = Args::parse_args(words.map(OsString::from).collect()).unwrap();
        let Some(Command::Setup { setup_args }) = &args.command else {
            panic!("not setup")
        };
        setup::run(&args, setup_args)
    };
    assert!(matches!(
        dispatch(&["--frozen", "--tools-only"]),
        Err(setup::Error::Unsupported(_))
    ));
    for flags in [&["--plan"][..], &["--check"], &["--__test-run"]] {
        assert!(matches!(dispatch(flags), Err(setup::Error::NotImplemented)));
    }
    assert!(
        Args::parse_args(
            ["turbo", "setup", "--frozen", "--update-lock"]
                .into_iter()
                .map(OsString::from)
                .collect()
        )
        .is_err()
    );
    let custom = f.root.join_component("custom.json");
    custom.create_with_contents(ENABLED).unwrap();
    assert!(matches!(
        dispatch(&["--root-turbo-json", custom.as_str()]),
        Err(setup::Error::NotImplemented)
    ));
    f.root
        .join_component("turbo.json")
        .create_with_contents("{}")
        .unwrap();
    assert!(matches!(
        dispatch(&["--update-lock"]),
        Err(setup::Error::Disabled)
    ));
    f.root
        .join_component("turbo.json")
        .create_with_contents("{invalid")
        .unwrap();
    assert!(matches!(
        dispatch(&[]),
        Err(setup::Error::Root(Error::TurboJson(_)))
    ));
    assert!(!f.root.join_component(".turbo").exists());
}

#[cfg(unix)]
#[test]
fn git_init_and_worktree_file_boundaries_change_nested_discovery() {
    let f = Fixture::new();
    let d = f.discovery();
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&f.nested)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!d.revalidate().unwrap());
    // Root inference is deliberately file-presence-based, not a Git probe. A
    // worktree's .git file is as authoritative as a clone's .git directory.
    fs::remove_dir_all(f.nested.join_component(".git")).unwrap();
    let again = f.discovery();
    f.nested
        .join_component(".git")
        .create_with_contents("gitdir: /not-probed/worktrees/web\n")
        .unwrap();
    assert!(!again.revalidate().unwrap());
    let explicit = f.capture(".", Some("turbo.json"));
    assert!(explicit.revalidate().unwrap());
    assert!(matches!(
        Discovery::capture_at(&f.args("apps/web", Some("turbo.json")), &f.root),
        Err(Error::Nested { .. })
    ));
}

#[cfg(unix)]
#[test]
fn cwd_and_config_symlink_retargeting_cannot_redirect_captured_context() {
    let f = Fixture::new();
    let second = f.root.join_component("second");
    second.create_dir_all().unwrap();
    second.join_component(".git").create_dir_all().unwrap();
    second
        .join_component("turbo.json")
        .create_with_contents(ENABLED)
        .unwrap();
    let link = f.root.join_component("linked");
    std::os::unix::fs::symlink(&f.nested, &link).unwrap();
    let d = f.capture("linked", None);
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    assert!(!d.revalidate().unwrap());
    let config_link = f.root.join_component("selected");
    std::os::unix::fs::symlink(f.root.join_component("turbo.json"), &config_link).unwrap();
    let custom = f.capture(".", Some("selected"));
    assert!(matches!(custom.revalidate(), Err(Error::UnsupportedConfig)));
    fs::remove_file(&config_link).unwrap();
    let config_link = f.root.join_component("turbo.jsonc");
    std::os::unix::fs::symlink(f.root.join_component("turbo.json"), &config_link).unwrap();
    let custom = f.capture(".", Some("turbo.jsonc"));
    assert!(custom.revalidate().unwrap());
    fs::remove_file(&config_link).unwrap();
    std::os::unix::fs::symlink(second.join_component("turbo.json"), &config_link).unwrap();
    assert!(matches!(custom.revalidate(), Err(Error::Outside { .. })));
}
