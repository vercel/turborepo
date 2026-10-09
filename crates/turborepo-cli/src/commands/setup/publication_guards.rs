//! Safety precursor: parser/discovery + existing library publication, not a
//! locally enabled setup mode. Fixture tools and lifecycle hooks never execute.
use std::path::Path;

use turborepo_setup::lock::{
    Lock, Snapshot,
    reconcile::{self, Mode},
};

use super::*;

fn state(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.is_dir() {
            result.extend(state(&path));
            Vec::new()
        } else if metadata.file_type().is_symlink() {
            fs::read_link(&path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else {
            fs::read(&path).unwrap()
        };
        result.push((path, bytes));
    }
    result.sort();
    result
}
fn check(f: &Fixture, d: &root::Discovery) -> std::io::Result<()> {
    f.owned
        .policy_at(&f.owned.root().join("apps/web/src"))
        .map_err(std::io::Error::other)?;
    provision::storage(f.owned.root()).map_err(std::io::Error::other)?;
    if !d.revalidate().map_err(std::io::Error::other)? {
        return Err(std::io::Error::other("original discovery drift"));
    }
    Ok(())
}
fn publish(
    f: &Fixture,
    desired: &Lock,
    hook: impl Fn(),
) -> Result<reconcile::Outcome, reconcile::Error> {
    let d = root::Discovery::capture(&f.args(FROZEN)).unwrap();
    let snapshot = Snapshot::capture(f.owned.root()).unwrap();
    reconcile::reconcile_checked(
        &snapshot,
        Mode::Local,
        false,
        |_| Ok(desired.clone()),
        |lock| {
            provision::plans(&snapshot, lock)
                .map(drop)
                .map_err(std::io::Error::other)
        },
        || {
            hook();
            check(f, &d)
        },
    )
}
#[test]
fn unsupported_unchanged_plans_fail_before_any_writes_in_both_consumers() {
    for case in 0..5 {
        let mut f = Fixture::new();
        match case {
            0 => {
                fs::write(f.owned.root().join("package.json"), "{}").unwrap();
                f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
                f.lock["tools"]["node"]["installation"] =
                    json!({"kind":"verify-system","executables":["node"]});
            }
            1 => {
                f.lock["tools"]["node"]["options"] = json!({"unsupported":["true"]});
            }
            2 => {
                let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap();
                let key = artifacts.keys().next().unwrap().clone();
                let old = artifacts.remove(&key).unwrap();
                artifacts.insert("windows-x64".into(), old);
            }
            3 => {
                f.lock["tools"]["pnpm"]["options"] = json!({"unsupported":["true"]});
            }
            _ => {
                f.lock["tools"]["node"]["version"] = json!("26.0.0");
                f.lock["tools"]["node"]["installation"] = serde_json::from_str(
                    &f.lock["tools"]["node"]["installation"]
                        .to_string()
                        .replace("24.0.0", "26.0.0"),
                )
                .unwrap();
            }
        }
        let desired = Lock::parse(f.lock.to_string().as_bytes()).unwrap();
        fs::write(
            f.owned.root().join("turbo.lock"),
            desired.canonical_bytes().unwrap(),
        )
        .unwrap();
        let before = state(f.owned.root());
        assert!(publish(&f, &desired, || {}).is_err(), "case {case}");
        assert_eq!(state(f.owned.root()), before);
        let u = Upstream::new(&f, false, None);
        assert!(f.run(&u, FROZEN).is_err());
        assert_eq!(state(f.owned.root()), before);
        assert_eq!(u.hits(), 0);
        f.no_execution();
    }
}
#[test]
fn resolved_candidate_missing_current_host_is_rejected_before_publication_or_storage() {
    let f = Fixture::new();
    fs::write(f.owned.root().join("package.json"), "{}").unwrap();
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    let (file, target) = if matches!(
        provision::platform().unwrap(),
        turborepo_setup::lock::Platform::LinuxX64Gnu
    ) {
        ("osx-arm64-tar", "darwin-arm64")
    } else {
        ("linux-x64", "linux-x64")
    };
    let routes = [
        (
            "/dist/index.json".into(),
            json!([{"version":"v24.0.0","npm":"11.6.1","lts":false,"files":[file]}])
                .to_string()
                .into_bytes(),
        ),
        (
            "/dist/v24.0.0/SHASUMS256.txt".into(),
            format!("{}  node-v24.0.0-{target}.tar.gz\n", "a".repeat(64)).into_bytes(),
        ),
    ];
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    let d = root::Discovery::capture(&f.args(FROZEN)).unwrap();
    let snapshot = Snapshot::capture(f.owned.root()).unwrap();
    let node = turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(u.origin())
        .unwrap();
    let registry =
        turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(
            u.origin(),
        )
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let before = state(f.owned.root());
    let validated = std::cell::Cell::new(false);
    let result = reconcile::reconcile_checked(
        &snapshot,
        Mode::Local,
        false,
        |request| {
            runtime
                .block_on(turborepo_setup::js_resolution::resolve(
                    request, &node, &registry,
                ))
                .map_err(|e| reconcile::Error::Resolution(e.to_string()))
        },
        |lock| {
            validated.set(true);
            assert!(matches!(
                provision::plans(&snapshot, lock),
                Err(Error::Node(
                    turborepo_setup::node_provision::Error::MissingTarget
                ))
            ));
            provision::plans(&snapshot, lock)
                .map(drop)
                .map_err(std::io::Error::other)
        },
        || check(&f, &d),
    );
    assert!(validated.get());
    assert!(result.is_err());
    assert_eq!(u.hits(), 2);
    assert_eq!(state(f.owned.root()), before);
    f.no_execution();
}
#[test]
fn workspace_symlinks_reject_before_writers_and_new_links_abort_after_staging() {
    for before_capture in [true, false] {
        let f = Fixture::new();
        let root = f.owned.root();
        fs::write(root.join("workspace.yaml"), "packages: ['apps/*']\n").unwrap();
        if before_capture {
            std::os::unix::fs::symlink("workspace.yaml", root.join("pnpm-workspace.yaml")).unwrap();
        }
        let before = state(root);
        if before_capture {
            let u = Upstream::new(&f, false, None);
            assert!(f.run(&u, FROZEN).is_err());
            assert_eq!(u.hits(), 0);
            assert_eq!(state(root), before);
            assert!(!root.join(".turbo").exists());
        } else {
            let desired = Lock::parse(f.lock.to_string().as_bytes()).unwrap();
            fs::remove_file(root.join("turbo.lock")).unwrap();
            let fired = std::cell::Cell::new(false);
            assert!(
                publish(&f, &desired, || {
                    if root.join(".turbo/setup-lock/staged").exists() && !fired.replace(true) {
                        std::os::unix::fs::symlink(
                            "workspace.yaml",
                            root.join("pnpm-workspace.yaml"),
                        )
                        .unwrap();
                        fs::write(
                            root.join("workspace.yaml"),
                            "packages: ['apps/*', 'packages/*']\n",
                        )
                        .unwrap();
                    }
                })
                .is_err()
            );
            assert!(fired.get());
            assert!(!root.join("turbo.lock").exists());
            assert!(f.manifest().is_none());
            assert!(!root.join(".turbo/setup-lock/staged").exists());
        }
        f.no_execution();
    }
}
#[test]
fn effective_git_targets_and_indirection_bytes_are_immutable_after_lock_staging() {
    for case in 0..6 {
        let f = Fixture::new();
        let root = f.owned.root();
        fs::rename(root.join(".git"), root.join("common")).unwrap();
        fs::create_dir(root.join("gitdir")).unwrap();
        fs::copy(root.join("common/HEAD"), root.join("gitdir/HEAD")).unwrap();
        fs::write(root.join("gitdir/commondir"), "../common\n").unwrap();
        fs::write(root.join(".git"), "gitdir: gitdir\n").unwrap();
        fs::write(
            root.join("gitdir/gitdir"),
            format!("{}\n", root.join(".git").display()),
        )
        .unwrap();
        let other = root.join("replacement");
        fs::create_dir(&other).unwrap();
        assert!(
            std::process::Command::new("git")
                .current_dir(&other)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        fs::rename(other.join(".git"), root.join("other-common")).unwrap();
        let worktree = root.parent().unwrap().join("worktree-link");
        if case == 5 {
            std::os::unix::fs::symlink(root, &worktree).unwrap();
            fs::write(root.join(".git"), "gitdir: common\n").unwrap();
            let config = fs::read_to_string(root.join("common/config")).unwrap();
            fs::write(
                root.join("common/config"),
                format!("{config}\n[core]\nworktree = {}\n", worktree.display()),
            )
            .unwrap();
        }
        let config_before = fs::read(root.join("common/config")).unwrap();
        let desired = Lock::parse(f.lock.to_string().as_bytes()).unwrap();
        fs::remove_file(root.join("turbo.lock")).unwrap();
        let original_gitfile = fs::read(root.join(".git")).unwrap();
        let fired = std::cell::Cell::new(false);
        let result = publish(&f, &desired, || {
            if !root.join(".turbo/setup-lock/staged").exists() || fired.replace(true) {
                return;
            }
            match case {
                0 => {
                    fs::rename(root.join("gitdir"), root.join("old-gitdir")).unwrap();
                    fs::create_dir(root.join("gitdir")).unwrap();
                    for name in ["HEAD", "commondir", "gitdir"] {
                        fs::copy(
                            root.join("old-gitdir").join(name),
                            root.join("gitdir").join(name),
                        )
                        .unwrap();
                    }
                }
                1 => {
                    fs::rename(root.join("common"), root.join("old-common")).unwrap();
                    fs::rename(root.join("other-common"), root.join("common")).unwrap();
                }
                2 => fs::write(root.join("gitdir/commondir"), "../other-common\n").unwrap(),
                3 => fs::write(
                    root.join("gitdir/gitdir"),
                    format!("{}\n", root.join("replacement/.git").display()),
                )
                .unwrap(),
                5 => {
                    fs::remove_file(&worktree).unwrap();
                    std::os::unix::fs::symlink(root.parent().unwrap(), &worktree).unwrap();
                    assert_eq!(fs::read(root.join("common/config")).unwrap(), config_before);
                    assert!(provision::storage(root).is_ok());
                }
                _ => {
                    fs::rename(root.join("common/config"), root.join("common/old-config")).unwrap();
                    fs::copy(root.join("common/old-config"), root.join("common/config")).unwrap();
                }
            }
        });
        assert!(fired.get(), "case {case}: {result:?}");
        assert!(result.is_err(), "case {case}");
        assert_eq!(fs::read(root.join(".git")).unwrap(), original_gitfile);
        assert!(!root.join("turbo.lock").exists());
        assert!(f.manifest().is_none());
        assert!(!root.join(".turbo/setup-lock/staged").exists());
        f.no_execution();
    }
}
#[test]
fn frozen_indirect_git_reuse_remains_supported_and_local_mode_stays_disabled() {
    let f = Fixture::new();
    let root = f.owned.root();
    fs::rename(root.join(".git"), root.join("gitdir")).unwrap();
    fs::write(root.join(".git"), "gitdir: gitdir\n").unwrap();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let before = state(root);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(state(root), before);
    assert_eq!(u.hits(), 3);
    assert!(matches!(
        f.run(&u, &["--tools-only", "--no-frozen"]),
        Err(Error::NotImplemented)
    ));
    assert_eq!(state(root), before);
    f.no_execution();
}
