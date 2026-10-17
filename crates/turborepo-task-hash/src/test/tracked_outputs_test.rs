use std::process::Command;

use turborepo_types::{DependencyOutputsInput, TaskOutputs};

use super::*;

fn git(root: &AbsoluteSystemPath, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &AbsoluteSystemPath, path: &str, contents: &str) {
    let path = root.join_unix_path(&RelativeUnixPathBuf::new(path).unwrap());
    path.ensure_dir().unwrap();
    path.create_with_contents(contents).unwrap();
}

async fn repository() -> (tempfile::TempDir, AbsoluteSystemPathBuf, PackageGraph) {
    let tmp = tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&root).await;
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    write(&root, "packages/app/src/input.txt", "source\n");
    write(&root, "packages/app/dist/LICENSE", "license v1\n");
    write(&root, "packages/app/dist/excluded.txt", "excluded v1\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    (tmp, root, graph)
}

fn definition(outputs: &[&str]) -> TaskDefinition {
    TaskDefinition {
        inputs: TaskInputs::new(vec!["src/**".to_owned(), "!dist/**".to_owned()]),
        outputs: TaskOutputs {
            inclusions: outputs.iter().map(|s| s.to_string()).collect(),
            exclusions: vec!["dist/excluded.txt".to_owned()],
        },
        ..TaskDefinition::default()
    }
}

fn inputs(
    root: &AbsoluteSystemPath,
    graph: &PackageGraph,
    definitions: &HashMap<TaskId<'static>, TaskDefinition>,
    indexed: bool,
    expanded: bool,
) -> PackageInputsHashes {
    let scm = SCM::new(root);
    let index = indexed.then(|| scm.build_repo_index_eager().unwrap());
    let tasks: Vec<_> = definitions.keys().cloned().map(TaskNode::Task).collect();
    PackageInputsHashes::calculate_file_hashes(
        &scm,
        tasks.iter(),
        graph,
        definitions,
        root,
        index.as_ref(),
        expanded,
    )
    .unwrap()
}

#[tokio::test]
async fn tracked_outputs_invalidate_explicit_inputs_without_hashing_generated_outputs() {
    for indexed in [false, true] {
        let (_tmp, root, graph) = repository().await;
        let task = TaskId::new("app", "build").into_owned();
        let definitions = HashMap::from([(task.clone(), definition(&["dist/**"]))]);
        let before = inputs(&root, &graph, &definitions, indexed, true);
        write(&root, "packages/app/dist/generated.txt", "generated\n");
        write(&root, "packages/app/dist/excluded.txt", "excluded v2\n");
        let generated = inputs(&root, &graph, &definitions, indexed, false);
        assert_eq!(before.hashes[&task], generated.hashes[&task]);
        write(&root, "packages/app/dist/LICENSE", "license v2\n");
        let modified = inputs(&root, &graph, &definitions, indexed, false);
        assert_ne!(before.hashes[&task], modified.hashes[&task]);
        git(&root, &["add", "packages/app/dist/LICENSE"]);
        git(&root, &["commit", "-m", "license v2"]);
        let committed = inputs(&root, &graph, &definitions, indexed, false);
        assert_eq!(modified.hashes[&task], committed.hashes[&task]);
        assert!(
            before.expanded_hashes[&task]
                .0
                .iter()
                .any(|(path, _)| path.as_str() == "dist/LICENSE")
        );
    }
}

#[tokio::test]
async fn tracked_outputs_do_not_share_hashes_between_different_output_selections() {
    let (_tmp, root, graph) = repository().await;
    let build = TaskId::new("app", "build").into_owned();
    let check = TaskId::new("app", "check").into_owned();
    let excluded = TaskId::new("app", "excluded").into_owned();
    let mut excluded_definition = definition(&["dist/**"]);
    excluded_definition
        .outputs
        .exclusions
        .push("dist/LICENSE".to_owned());
    let definitions = HashMap::from([
        (build.clone(), definition(&["dist/**"])),
        (check.clone(), definition(&["other/**"])),
        (excluded.clone(), excluded_definition),
    ]);
    let before = inputs(&root, &graph, &definitions, true, false);
    write(&root, "packages/app/dist/LICENSE", "changed\n");
    let after = inputs(&root, &graph, &definitions, true, false);
    assert_ne!(before.hashes[&build], after.hashes[&build]);
    assert_eq!(before.hashes[&check], after.hashes[&check]);
    assert_eq!(before.hashes[&excluded], after.hashes[&excluded]);
}

#[tokio::test]
async fn tracked_outputs_include_staged_ignored_files_and_drop_deleted_files() {
    for indexed in [false, true] {
        let (_tmp, root, graph) = repository().await;
        let task = TaskId::new("app", "build").into_owned();
        let definitions = HashMap::from([(task.clone(), definition(&["dist"]))]);
        write(&root, ".gitignore", "packages/app/dist/staged.txt\n");
        write(&root, "packages/app/dist/staged.txt", "staged v1\n");
        let before = inputs(&root, &graph, &definitions, indexed, false);
        git(&root, &["add", "-f", "packages/app/dist/staged.txt"]);
        let staged = inputs(&root, &graph, &definitions, indexed, false);
        assert_ne!(before.hashes[&task], staged.hashes[&task]);
        write(&root, "packages/app/dist/staged.txt", "working tree v2\n");
        let modified = inputs(&root, &graph, &definitions, indexed, false);
        assert_ne!(staged.hashes[&task], modified.hashes[&task]);
        std::fs::remove_file(root.join_components(&["packages", "app", "dist", "staged.txt"]))
            .unwrap();
        let deleted = inputs(&root, &graph, &definitions, indexed, false);
        assert_eq!(before.hashes[&task], deleted.hashes[&task]);
        git(
            &root,
            &["rm", "--cached", "-f", "packages/app/dist/staged.txt"],
        );
        write(&root, "packages/app/dist/staged.txt", "no longer tracked\n");
        let untracked = inputs(&root, &graph, &definitions, indexed, false);
        assert_eq!(before.hashes[&task], untracked.hashes[&task]);
    }
}

#[tokio::test]
async fn tracked_outputs_can_reach_outside_the_package() {
    let (_tmp, root, graph) = repository().await;
    write(&root, "shared/LICENSE", "shared v1\n");
    git(&root, &["add", "shared/LICENSE"]);
    let task = TaskId::new("app", "build").into_owned();
    let definitions = HashMap::from([(task.clone(), definition(&["../../shared/**"]))]);
    for indexed in [false, true] {
        write(&root, "shared/LICENSE", "shared v1\n");
        let before = inputs(&root, &graph, &definitions, indexed, false);
        write(&root, "shared/LICENSE", "shared v2\n");
        let after = inputs(&root, &graph, &definitions, indexed, false);
        assert_ne!(before.hashes[&task], after.hashes[&task]);
    }
}

fn deferred_hash(
    root: &AbsoluteSystemPath,
    graph: &PackageGraph,
    task: &TaskId<'static>,
    definition: &TaskDefinition,
) -> String {
    let definitions = HashMap::from([(task.clone(), definition.clone())]);
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    let hasher = TaskHasher::new(
        inputs(root, graph, &definitions, true, false),
        &run_opts,
        &env,
        "global-hash",
        root,
        EnvironmentVariableMap::default(),
        &[],
    );
    hasher
        .calculate_task_hash_with_deferred_inputs(
            TaskHashRequest {
                task_id: task,
                task_definition: definition,
                task_env_mode: EnvMode::Strict,
                package_context: &graph
                    .package_task_context(&PackageName::from("app"))
                    .unwrap(),
                dependency_set: &[],
                telemetry: PackageTaskEventBuilder::new("app", "build"),
            },
            DeferredHashInputs {
                scm: &SCM::new(root),
                repo_root: root,
                repo_index: None,
                dependency_output_hashes: None,
                dependency_output_producers: &HashSet::new(),
            },
        )
        .unwrap()
}

#[tokio::test]
async fn tracked_outputs_are_inputs_for_deferred_tasks() {
    for task_inputs in [
        TaskInputs {
            eager: false,
            jit_globs: vec!["src/**".to_owned()],
            ..TaskInputs::default()
        },
        TaskInputs {
            globs: vec!["src/**".to_owned()],
            dependency_outputs: Some(DependencyOutputsInput {
                from: None,
                globs: vec!["**".to_owned()],
            }),
            ..TaskInputs::default()
        },
    ] {
        let (_tmp, root, graph) = repository().await;
        let task = TaskId::new("app", "build").into_owned();
        let mut definition = definition(&["dist/**"]);
        definition.inputs = task_inputs;
        let before = deferred_hash(&root, &graph, &task, &definition);
        write(&root, "packages/app/dist/generated.txt", "generated\n");
        assert_eq!(before, deferred_hash(&root, &graph, &task, &definition));
        write(&root, "packages/app/dist/LICENSE", "changed\n");
        assert_ne!(before, deferred_hash(&root, &graph, &task, &definition));
    }
}
