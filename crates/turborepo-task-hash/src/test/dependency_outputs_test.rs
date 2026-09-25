use super::*;

fn write_task_output(repo_root: &AbsoluteSystemPathBuf, package: &str, contents: &str) {
    let output_file = repo_root.join_components(&["packages", package, "dist", "generated.txt"]);
    output_file.ensure_dir().unwrap();
    output_file.create_with_contents(contents).unwrap();
}

fn dependency_output_hashes(
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    packages: &[&str],
) -> Arc<FileHashes> {
    let scm = SCM::new(repo_root);
    let mut combined = BTreeMap::new();
    for package in packages {
        let package = PackageName::from(*package);
        let context = graph.package_task_context(&package).unwrap();
        let output_hashes = file_hashes_for_inputs(
            &scm,
            repo_root,
            context.directory(),
            &["dist/**"],
            false,
            None,
        )
        .unwrap();
        for (path, hash) in output_hashes
            .0
            .iter()
            .filter(|(path, _)| path.as_str().starts_with("dist/"))
        {
            let absolute_path = repo_root.resolve(context.directory()).join_unix_path(path);
            let repo_relative_path =
                AnchoredSystemPathBuf::relative_path_between(repo_root, &absolute_path).to_unix();
            combined.insert(repo_relative_path, *hash);
        }
    }
    Arc::new(FileHashes(combined.into_iter().collect()))
}

fn consumer_hash_with_outputs(
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    dependency_outputs: Arc<FileHashes>,
    producers: &[(&TaskId<'static>, &str)],
) -> String {
    let consumer = TaskId::new("app", "build").into_owned();
    let eager_inputs = Arc::new(FileHashes(Vec::new()));
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    let hasher = TaskHasher::new(
        PackageInputsHashes {
            hashes: HashMap::from([(consumer.clone(), eager_inputs.as_ref().clone().hash())]),
            expanded_hashes: HashMap::from([(consumer.clone(), eager_inputs)]),
        },
        &run_opts,
        &env,
        "global-hash",
        repo_root,
        EnvironmentVariableMap::default(),
        &[],
    );

    let tracker = hasher.task_hash_tracker();
    let mut dependency_nodes = Vec::with_capacity(producers.len());
    let mut selected_producers = HashSet::with_capacity(producers.len());
    for (producer, producer_hash) in producers {
        let producer = (*producer).clone();
        tracker.insert_hash(
            producer.clone(),
            DetailedMap::default(),
            Arc::from(*producer_hash),
            None,
        );
        selected_producers.insert(producer.clone());
        dependency_nodes.push(TaskNode::Task(producer));
    }
    let dependency_set = dependency_nodes.iter().collect::<Vec<_>>();
    let package = PackageName::from("app");
    let package_context = graph.package_task_context(&package).unwrap();

    hasher
        .calculate_task_hash_with_deferred_inputs(
            &consumer,
            &TaskDefinition::default(),
            EnvMode::Strict,
            &package_context,
            &dependency_set,
            PackageTaskEventBuilder::new("app", "build"),
            &SCM::new(repo_root),
            repo_root,
            None,
            Some(dependency_outputs),
            &selected_producers,
        )
        .unwrap()
}

#[tokio::test]
async fn dependency_output_file_changes_consumer_task_hash() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let producer = TaskId::new("other", "build").into_owned();

    write_task_output(&repo_root, "other", "generated-v1\n");
    let first = consumer_hash_with_outputs(
        &repo_root,
        &graph,
        dependency_output_hashes(&repo_root, &graph, &["other"]),
        &[(&producer, "producer-task-hash")],
    );

    write_task_output(&repo_root, "other", "generated-v2\n");
    let second = consumer_hash_with_outputs(
        &repo_root,
        &graph,
        dependency_output_hashes(&repo_root, &graph, &["other"]),
        &[(&producer, "producer-task-hash")],
    );

    assert_ne!(
        first, second,
        "changed dependency output must invalidate app"
    );
}

#[tokio::test]
async fn dependency_output_hash_replaces_selected_dependency_task_hash() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    write_task_output(&repo_root, "other", "stable-output\n");
    let dependency_outputs = dependency_output_hashes(&repo_root, &graph, &["other"]);
    let producer = TaskId::new("other", "build").into_owned();

    let first = consumer_hash_with_outputs(
        &repo_root,
        &graph,
        dependency_outputs.clone(),
        &[(&producer, "producer-task-hash-v1")],
    );
    let second = consumer_hash_with_outputs(
        &repo_root,
        &graph,
        dependency_outputs,
        &[(&producer, "producer-task-hash-v2")],
    );

    assert_eq!(
        first, second,
        "selected dependency task hashes are replaced by their output hashes"
    );
}

#[tokio::test]
async fn dependency_output_hashes_distinguish_identical_paths_from_different_packages() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    write_task_output(&repo_root, "app", "app-output-v1\n");
    write_task_output(&repo_root, "other", "other-output-v1\n");

    let dependency_outputs = dependency_output_hashes(&repo_root, &graph, &["app", "other"]);
    let paths = dependency_outputs
        .0
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "packages/app/dist/generated.txt",
            "packages/other/dist/generated.txt"
        ]
    );

    let app_producer = TaskId::new("app", "codegen").into_owned();
    let other_producer = TaskId::new("other", "build").into_owned();
    let first = consumer_hash_with_outputs(
        &repo_root,
        &graph,
        dependency_outputs,
        &[
            (&app_producer, "app-producer-hash"),
            (&other_producer, "other-producer-hash"),
        ],
    );

    write_task_output(&repo_root, "other", "other-output-v2\n");
    let second = consumer_hash_with_outputs(
        &repo_root,
        &graph,
        dependency_output_hashes(&repo_root, &graph, &["app", "other"]),
        &[
            (&app_producer, "app-producer-hash"),
            (&other_producer, "other-producer-hash"),
        ],
    );

    assert_ne!(
        first, second,
        "each package's output path must hash separately"
    );
}
