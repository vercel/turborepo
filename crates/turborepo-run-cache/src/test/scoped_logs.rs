use std::collections::HashMap;

use turborepo_repository::toolchain::{
    DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackage,
    DiscoveredPackageScopes, DiscoveredPackages, RepositoryContributor, ToolchainId, WorkspaceRoot,
};

use super::*;

struct Contributor {
    root: AbsoluteSystemPathBuf,
    id: ToolchainId,
    name: &'static str,
    manifest: &'static str,
    directory: &'static str,
}

impl RepositoryContributor for Contributor {
    fn id(&self) -> ToolchainId {
        self.id.clone()
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async {
            Ok(DiscoveredPackages::new(
                vec![DiscoveredPackage::package(
                    Some(self.name.to_string()),
                    PackageJson::default(),
                    self.root
                        .join_components(&["packages", self.directory, self.manifest]),
                )],
                vec![WorkspaceRoot::new(self.id.as_str(), self.root.clone())],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async {
            let full = self.discover_packages().await?;
            Ok(DiscoveredPackageScopes::from_full_observation(
                full.packages(),
                full.workspace_roots(),
            ))
        })
    }
}

const NAMES: [&str; 3] = ["app", "example.com/go", "rust:*?[native]"];

async fn graph(root: &AbsoluteSystemPathBuf) -> PackageGraph {
    // Reuse the ordinary JavaScript fixture, then add two native scopes without
    // invoking external toolchains. Keep the native names deliberately unsafe
    // as literal filesystem components.
    javascript_graph(root, "packages").await;
    assemble_graph(root, "app").await
}

async fn assemble_graph(root: &AbsoluteSystemPathBuf, directory: &'static str) -> PackageGraph {
    let builder = PackageGraph::builder(
        root,
        PackageJson::load(&root.join_component("package.json")).unwrap(),
    )
    .with_contributor(Arc::new(Contributor {
        root: root.clone(),
        id: ToolchainId::GO,
        name: NAMES[1],
        manifest: "go.mod",
        directory,
    }))
    .with_contributor(Arc::new(Contributor {
        root: root.clone(),
        id: ToolchainId::RUST,
        name: NAMES[2],
        manifest: "Cargo.toml",
        directory,
    }));
    let (inventory, mut plan) = builder.build_lazy().await.unwrap().into_parts();
    for name in NAMES {
        assert_eq!(
            inventory
                .package_task_context(&PackageName::from(name))
                .unwrap()
                .log_namespace(),
            Some(name)
        );
    }
    assert_eq!(
        inventory
            .package_task_context(&PackageName::Root)
            .unwrap()
            .log_namespace(),
        None
    );
    let loaded = plan
        .load(&HashSet::from([ToolchainId::GO, ToolchainId::RUST]))
        .await
        .unwrap();
    for name in NAMES {
        assert_eq!(
            loaded
                .package_task_context(&PackageName::from(name))
                .unwrap()
                .log_namespace(),
            Some(name)
        );
    }
    loaded
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_scoped_logs_save_restore_and_replay_independently() {
    for broad_outputs in [false, true] {
        let tmp = tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let graph = graph(&root).await;
        let cache = run_cache(&root);
        let definition = TaskDefinition {
            output_logs: OutputLogsMode::Full,
            outputs: TaskOutputs {
                inclusions: if broad_outputs {
                    vec![".turbo/**".to_string()]
                } else {
                    Vec::new()
                },
                exclusions: Vec::new(),
            },
            ..Default::default()
        };
        let mut tasks: Vec<_> = NAMES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                cache
                    .task_cache(
                        &definition,
                        &graph
                            .package_task_context(&PackageName::from(*name))
                            .unwrap(),
                        TaskId::new(name, "build").into_owned(),
                        &format!("scoped-log-{index}"),
                    )
                    .unwrap()
            })
            .collect();
        let paths: Vec<_> = tasks
            .iter()
            .map(|task| task.log_file_path.clone())
            .collect();
        assert_eq!(paths.iter().collect::<HashSet<_>>().len(), 3);
        let contents: Vec<_> = (0..3)
            .map(|index| {
                (0..4)
                    .map(|line| format!("owner-{index}: line-{line}\n"))
                    .collect::<String>()
            })
            .collect();

        // Open every file before writing anything: the old implementation would
        // give three independent file offsets into the same truncated log.
        let writers: Vec<_> = tasks
            .iter()
            .map(|task| task.output_writer(std::io::sink()).unwrap())
            .collect();
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for (index, mut writer) in writers.into_iter().enumerate() {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for line in 0..4 {
                        writeln!(writer, "owner-{index}: line-{line}").unwrap();
                        writer.flush().unwrap();
                    }
                });
            }
        });
        for (path, expected) in paths.iter().zip(&contents) {
            assert_eq!(path.read_to_string().unwrap(), *expected);
        }
        let log_dir = root.join_components(&["packages", "app", ".turbo"]);
        // A user-owned .log in the reserved directory must not be mistaken for
        // a managed digest-named task log when output globs are broad.
        let notes = log_dir.join_components(&["task-logs", "notes.log"]);
        notes.create_with_contents("user artifact\n").unwrap();
        for (index, task) in tasks.iter_mut().enumerate() {
            task.save_outputs(
                Duration::from_millis(1),
                &PackageTaskEventBuilder::new(NAMES[index], "build"),
            )
            .await
            .unwrap();
        }
        cache.cache.wait().await.unwrap();
        assert!(cache.warnings.lock().unwrap().is_empty());

        // Restore each archive into an empty log directory. Restoring all entries
        // together could hide an archive accidentally containing a peer's log.
        for (index, task) in tasks.iter_mut().enumerate() {
            std::fs::remove_dir_all(log_dir.as_std_path()).unwrap();
            let (sink, mut handle) = recording_task_handle();
            assert!(
                task.restore_outputs(
                    &mut handle,
                    None,
                    &PackageTaskEventBuilder::new(NAMES[index], "build")
                )
                .await
                .unwrap()
                .is_some()
            );
            assert_eq!(paths[index].read_to_string().unwrap(), contents[index]);
            assert!(sink.output_string().contains(&contents[index]));
            for (other, path) in paths.iter().enumerate() {
                if other != index {
                    assert!(!path.exists(), "archive {index} restored peer {other}");
                    assert!(!sink.output_string().contains(&format!("owner-{other}:")));
                }
            }
            assert_eq!(notes.exists(), broad_outputs);
        }

        // Shared-cache concurrent restoration must also preserve every identity.
        std::fs::remove_dir_all(log_dir.as_std_path()).unwrap();
        let mut restorations = tokio::task::JoinSet::new();
        for (index, mut task) in tasks.into_iter().enumerate() {
            let expected = contents[index].clone();
            restorations.spawn(async move {
                let (sink, mut handle) = recording_task_handle();
                assert!(
                    task.restore_outputs(
                        &mut handle,
                        None,
                        &PackageTaskEventBuilder::new(NAMES[index], "build")
                    )
                    .await
                    .unwrap()
                    .is_some()
                );
                assert_eq!(task.log_file_path.read_to_string().unwrap(), expected);
                assert!(sink.output_string().contains(&expected));
            });
        }
        while let Some(result) = restorations.join_next().await {
            result.unwrap();
        }
        for (path, expected) in paths.iter().zip(&contents) {
            assert_eq!(path.read_to_string().unwrap(), *expected);
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn aliased_scope_directories_have_distinct_physical_logs() {
    let tmp = tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path())
        .unwrap()
        .to_realpath()
        .unwrap();
    javascript_graph(&root, "packages").await;
    // Do not discover the JavaScript manifest twice through packages/*.
    root.join_component("package.json")
        .create_with_contents(
            serde_json::to_string(&json!({
                "name": "root",
                "packageManager": "npm@10.0.0",
                "workspaces": ["packages/app"]
            }))
            .unwrap(),
        )
        .unwrap();
    std::os::unix::fs::symlink("app", root.join_components(&["packages", "alias"])).unwrap();
    // This also asserts namespaces in the lazy inventory, before native scopes
    // are loaded. Canonical directory grouping must apply at both stages.
    let graph = assemble_graph(&root, "alias").await;
    let cache = run_cache(&root);
    let definition = TaskDefinition::default();
    let tasks: Vec<_> = NAMES
        .iter()
        .map(|name| {
            cache
                .task_cache(
                    &definition,
                    &graph
                        .package_task_context(&PackageName::from(*name))
                        .unwrap(),
                    TaskId::new(name, "build").into_owned(),
                    "aliased-scopes",
                )
                .unwrap()
        })
        .collect();
    // Open all writers first so aliases of a shared file cannot pass by merely
    // reading each log immediately after its write.
    let writers: Vec<_> = tasks
        .iter()
        .map(|task| task.output_writer(std::io::sink()).unwrap())
        .collect();
    for (name, mut writer) in NAMES.iter().zip(writers) {
        writeln!(writer, "log from {name}").unwrap();
        writer.flush().unwrap();
    }
    let physical_paths: HashSet<_> = tasks
        .iter()
        .map(|task| task.log_file_path.to_realpath().unwrap())
        .collect();
    assert_eq!(physical_paths.len(), NAMES.len());
    let physical_log_dir = root.join_components(&["packages", "app", ".turbo", "task-logs"]);
    for (name, task) in NAMES.iter().zip(&tasks) {
        let physical = task.log_file_path.to_realpath().unwrap();
        assert_eq!(
            physical.as_std_path().parent().unwrap(),
            physical_log_dir.as_std_path()
        );
        let filename = physical
            .as_std_path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let digest = filename.strip_suffix(".log").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            physical.read_to_string().unwrap(),
            format!("log from {name}\n")
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_output_directory_does_not_archive_or_overwrite_peer_logs() {
    for literal_outputs in [false, true] {
        let tmp = tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        let graph = graph(&root).await;
        let cache = run_cache(&root);
        let definition = TaskDefinition {
            output_logs: OutputLogsMode::Full,
            outputs: TaskOutputs {
                inclusions: vec!["logs/**".to_string()],
                exclusions: Vec::new(),
            },
            ..Default::default()
        };
        let mut tasks: Vec<_> = NAMES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                cache
                    .task_cache(
                        &definition,
                        &graph
                            .package_task_context(&PackageName::from(*name))
                            .unwrap(),
                        TaskId::new(name, "build").into_owned(),
                        &format!("symlinked-log-{index}"),
                    )
                    .unwrap()
            })
            .collect();
        let paths: Vec<_> = tasks
            .iter()
            .map(|task| task.log_file_path.clone())
            .collect();
        if literal_outputs {
            // Literal paths bypass manual expansion of symlinked output globs. Use
            // an actual peer's digest so filtering must recognize its canonical
            // managed-log path, not just the lexical logs/ alias.
            let peer_filename = paths[1]
                .as_std_path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            let definition = TaskDefinition {
                outputs: TaskOutputs {
                    inclusions: vec!["logs".to_string(), format!("logs/{peer_filename}")],
                    exclusions: Vec::new(),
                },
                ..definition
            };
            tasks[0] = cache
                .task_cache(
                    &definition,
                    &graph
                        .package_task_context(&PackageName::from(NAMES[0]))
                        .unwrap(),
                    TaskId::new(NAMES[0], "build"),
                    "symlinked-log-0",
                )
                .unwrap();
            assert_eq!(tasks[0].log_file_path, paths[0]);
        }
        for (index, task) in tasks.iter().enumerate() {
            let mut writer = task.output_writer(std::io::sink()).unwrap();
            writeln!(writer, "original log {index}").unwrap();
            writer.flush().unwrap();
        }
        let app = root.join_components(&["packages", "app"]);
        std::os::unix::fs::symlink(".turbo/task-logs", app.join_component("logs")).unwrap();
        // For the glob variant, a non-managed file proves that the symlink's
        // contents really are followed. The literal variant does not select it.
        let notes = app.join_components(&[".turbo", "task-logs", "notes.log"]);
        notes.create_with_contents("user artifact\n").unwrap();
        let task = &mut tasks[0];
        let telemetry = PackageTaskEventBuilder::new(NAMES[0], "build");
        task.save_outputs(Duration::from_millis(1), &telemetry)
            .await
            .unwrap();
        cache.cache.wait().await.unwrap();

        // Restore without any logs present: peer bytes must not be in the archive,
        // either under their managed paths or under the logs/ alias.
        for path in &paths {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_file(&notes).unwrap();
        let (sink, mut handle) = recording_task_handle();
        assert!(
            task.restore_outputs(&mut handle, None, &telemetry)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(paths[0].read_to_string().unwrap(), "original log 0\n");
        assert!(sink.output_string().contains("original log 0\n"));
        if literal_outputs {
            assert!(!notes.exists());
        } else {
            assert_eq!(notes.read_to_string().unwrap(), "user artifact\n");
        }
        for path in &paths[1..] {
            assert!(
                !path.exists(),
                "restored peer log: {path} (literal outputs: {literal_outputs})"
            );
        }

        // Restore again with newer peer logs already in place. An archived alias
        // must not overwrite the underlying peer file.
        for (index, path) in paths.iter().enumerate().skip(1) {
            path.create_with_contents(format!("newer peer {index}\n"))
                .unwrap();
        }
        std::fs::remove_file(&paths[0]).unwrap();
        let (_, mut handle) = recording_task_handle();
        assert!(
            task.restore_outputs(&mut handle, None, &telemetry)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(paths[0].read_to_string().unwrap(), "original log 0\n");
        for (index, path) in paths.iter().enumerate().skip(1) {
            assert_eq!(
                path.read_to_string().unwrap(),
                format!("newer peer {index}\n"),
                "literal outputs: {literal_outputs}"
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct WatchRegistration {
    hash: String,
    inclusions: Vec<String>,
    exclusions: Vec<String>,
}

struct LogSnapshot {
    glob: String,
    bytes: Option<Vec<u8>>,
}

// The primary registration models unchanged ordinary outputs with .turbo/**
// excluded. Only the independent log registration can detect log changes. Byte
// snapshots avoid sleeps, filesystem timestamp resolution, and daemon races.
struct SnapshotOutputWatcher {
    root: AbsoluteSystemPathBuf,
    snapshots: Mutex<HashMap<String, LogSnapshot>>,
    registrations: Mutex<Vec<WatchRegistration>>,
    checks: Mutex<Vec<String>>,
}

impl SnapshotOutputWatcher {
    fn read_log(&self, glob: &str) -> Option<Vec<u8>> {
        match std::fs::read(self.root.as_std_path().join(glob)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("cannot snapshot {glob}: {error}"),
        }
    }

    fn assert_registration_pair(&self, task: &TaskCache) {
        let log = AnchoredSystemPathBuf::relative_path_between(&self.root, &task.log_file_path)
            .to_unix()
            .to_string();
        assert_eq!(
            std::mem::take(&mut *self.registrations.lock().unwrap()),
            vec![
                WatchRegistration {
                    hash: task.hash.clone(),
                    inclusions: task.repo_relative_globs.inclusions.clone(),
                    exclusions: vec!["packages/app/.turbo/**".to_string()],
                },
                WatchRegistration {
                    hash: format!("{}-task-log", task.hash),
                    inclusions: vec![log],
                    exclusions: Vec::new(),
                },
            ]
        );
    }

    fn assert_check_pair(&self, hash: &str) {
        assert_eq!(
            std::mem::take(&mut *self.checks.lock().unwrap()),
            vec![hash.to_string(), format!("{hash}-task-log")]
        );
    }
}

impl OutputWatcher for SnapshotOutputWatcher {
    fn get_changed_outputs(
        &self,
        hash: String,
        output_globs: Vec<String>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HashSet<String>, OutputWatcherError>> + Send>,
    > {
        self.checks.lock().unwrap().push(hash.clone());
        let changed = if hash.ends_with("-task-log") {
            assert_eq!(output_globs.len(), 1);
            let snapshots = self.snapshots.lock().unwrap();
            let unchanged = snapshots.get(&hash).is_some_and(|snapshot| {
                snapshot.glob == output_globs[0]
                    && snapshot.bytes == self.read_log(&output_globs[0])
            });
            if unchanged {
                HashSet::new()
            } else {
                output_globs.into_iter().collect()
            }
        } else {
            HashSet::new()
        };
        Box::pin(async move { Ok(changed) })
    }

    fn notify_outputs_written(
        &self,
        hash: String,
        output_globs: Vec<String>,
        output_exclusion_globs: Vec<String>,
        _time_saved: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OutputWatcherError>> + Send>>
    {
        if hash.ends_with("-task-log") {
            assert_eq!(output_globs.len(), 1);
            assert!(output_exclusion_globs.is_empty());
            self.snapshots.lock().unwrap().insert(
                hash.clone(),
                LogSnapshot {
                    glob: output_globs[0].clone(),
                    bytes: self.read_log(&output_globs[0]),
                },
            );
        } else {
            assert_eq!(output_exclusion_globs, vec!["packages/app/.turbo/**"]);
        }
        self.registrations.lock().unwrap().push(WatchRegistration {
            hash,
            inclusions: output_globs,
            exclusions: output_exclusion_globs,
        });
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn watcher_tracks_excluded_scoped_log_deletion_and_hash_switches() {
    let tmp = tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path())
        .unwrap()
        .to_realpath()
        .unwrap();
    let graph = graph(&root).await;
    let watcher = Arc::new(SnapshotOutputWatcher {
        root: root.clone(),
        snapshots: Mutex::new(HashMap::new()),
        registrations: Mutex::new(Vec::new()),
        checks: Mutex::new(Vec::new()),
    });
    let mut cache = run_cache(&root);
    Arc::get_mut(&mut cache).unwrap().output_watcher = Some(watcher.clone());
    let definition = TaskDefinition {
        output_logs: OutputLogsMode::Full,
        outputs: TaskOutputs {
            inclusions: vec!["dist/**".to_string()],
            exclusions: vec![".turbo/**".to_string()],
        },
        ..Default::default()
    };
    let artifact = root.join_components(&["packages", "app", "dist", "result.txt"]);
    artifact.ensure_dir().unwrap();
    artifact
        .create_with_contents("unchanged artifact\n")
        .unwrap();
    let context = graph
        .package_task_context(&PackageName::from(NAMES[0]))
        .unwrap();
    let mut task_a = cache
        .task_cache(
            &definition,
            &context,
            TaskId::new(NAMES[0], "build"),
            "hash-A",
        )
        .unwrap();
    let mut task_b = cache
        .task_cache(
            &definition,
            &context,
            TaskId::new(NAMES[0], "build"),
            "hash-B",
        )
        .unwrap();
    assert_eq!(task_a.log_file_path, task_b.log_file_path);
    let telemetry = PackageTaskEventBuilder::new(NAMES[0], "build");

    {
        let mut writer = task_a.output_writer(std::io::sink()).unwrap();
        writeln!(writer, "log for A").unwrap();
        writer.flush().unwrap();
    }
    task_a
        .save_outputs(Duration::from_millis(1), &telemetry)
        .await
        .unwrap();
    cache.cache.wait().await.unwrap();
    watcher.assert_registration_pair(&task_a);

    // The primary watcher reports unchanged outputs, but deleting the excluded
    // log must still fetch the archive and re-register both watches.
    std::fs::remove_file(&task_a.log_file_path).unwrap();
    let (sink, mut handle) = recording_task_handle();
    assert!(
        task_a
            .restore_outputs(&mut handle, None, &telemetry)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        task_a.log_file_path.read_to_string().unwrap(),
        "log for A\n"
    );
    assert!(sink.output_string().contains("log for A\n"));
    assert!(!sink.output_string().contains("outputs already on disk"));
    watcher.assert_check_pair("hash-A");
    watcher.assert_registration_pair(&task_a);

    // A restored snapshot is now current: prove the mock is not simply forcing
    // every restore, and that registration on restore captured the file bytes.
    let (sink, mut handle) = recording_task_handle();
    assert!(
        task_a
            .restore_outputs(&mut handle, None, &telemetry)
            .await
            .unwrap()
            .is_some()
    );
    assert!(sink.output_string().contains("outputs already on disk"));
    watcher.assert_check_pair("hash-A");
    assert!(watcher.registrations.lock().unwrap().is_empty());

    {
        let mut writer = task_b.output_writer(std::io::sink()).unwrap();
        writeln!(writer, "log for B").unwrap();
        writer.flush().unwrap();
    }
    task_b
        .save_outputs(Duration::from_millis(1), &telemetry)
        .await
        .unwrap();
    cache.cache.wait().await.unwrap();
    watcher.assert_registration_pair(&task_b);
    assert_eq!(
        task_b.log_file_path.read_to_string().unwrap(),
        "log for B\n"
    );

    // A -> B -> A uses the same physical log, but A's watcher snapshot must not
    // be replaced by B's. Otherwise A would incorrectly replay B's bytes.
    for (task, expected, unexpected) in [
        (&mut task_a, "log for A\n", "log for B\n"),
        (&mut task_b, "log for B\n", "log for A\n"),
    ] {
        let (sink, mut handle) = recording_task_handle();
        assert!(
            task.restore_outputs(&mut handle, None, &telemetry)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(task.log_file_path.read_to_string().unwrap(), expected);
        assert!(sink.output_string().contains(expected));
        assert!(!sink.output_string().contains(unexpected));
        assert!(!sink.output_string().contains("outputs already on disk"));
        watcher.assert_check_pair(&task.hash);
        watcher.assert_registration_pair(task);
    }
    assert_eq!(artifact.read_to_string().unwrap(), "unchanged artifact\n");
}

#[tokio::test]
async fn scoped_errors_only_logs_remain_isolated_without_caching() {
    let tmp = tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path())
        .unwrap()
        .to_realpath()
        .unwrap();
    let graph = graph(&root).await;
    let cache = run_cache(&root);
    let definition = TaskDefinition {
        cache: false,
        output_logs: OutputLogsMode::ErrorsOnly,
        ..Default::default()
    };
    for name in NAMES {
        let task = cache
            .task_cache(
                &definition,
                &graph
                    .package_task_context(&PackageName::from(name))
                    .unwrap(),
                TaskId::new(name, "build").into_owned(),
                "uncached",
            )
            .unwrap();
        let mut writer = task.output_writer(std::io::sink()).unwrap();
        writeln!(writer, "error from {name}").unwrap();
        writer.flush().unwrap();
        drop(writer);
        let (sink, mut handle) = recording_task_handle();
        task.on_error(&mut handle, None).unwrap();
        assert!(sink.output_string().contains(&format!("error from {name}")));
        assert!(task.exists().await.unwrap().is_none());
    }
}
