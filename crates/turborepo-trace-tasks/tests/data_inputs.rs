//! Which inputs a task only reads, as opposed to the files it runs or compiles
//! into its outputs ([`Membership::data`], [`TaskSpec::data_inputs`]). Fixture:
//! `fixtures/tasks/data-inputs/` (see its README), in the shape of a large
//! internal monorepo where the uncertainty of a script that a `tsc -b`
//! type-check merely lists put every importer of the package on the "always
//! selected" floor.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_tasks::{
    Options, Task, TaskSet, discover,
    sources::Workspace,
    spec::{Membership, OutputKind, TaskId, TaskSpec},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/data-inputs")
        .canonicalize()
        .expect("fixture dir")
}

fn task<'a>(set: &'a TaskSet, id: &str) -> &'a Task {
    set.get(&id.parse::<TaskId>().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"))
}

/// `(dir/pattern, reason, read-only)` of every membership of task `id`.
fn memberships(set: &TaskSet, root: &Path, id: &str) -> Vec<(String, String, bool)> {
    let t = task(set, id);
    let show = |m: &Membership| {
        let dir = m.dir.strip_prefix(root).unwrap().to_string_lossy();
        (format!("{dir}/{}", m.pattern), m.reason.clone(), m.data)
    };
    let mut out: Vec<_> = t.spec.memberships.iter().map(show).collect();
    // A part's memberships are the task's as well: they must agree.
    for p in &t.spec.parts {
        for m in &p.memberships {
            assert!(out.contains(&show(m)), "{id}: part membership {m:?}");
        }
    }
    out.sort();
    out
}

fn m(path: &str, reason: &str, data: bool) -> (String, String, bool) {
    (path.to_owned(), reason.to_owned(), data)
}

fn rel(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_type_check_only_reads_its_project_and_its_declared_inputs() {
    let root = fixture();
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());

    // `tsc -b` over a declaration-only project, with turbo `inputs:
    // ["$TURBO_DEFAULT$", "$TURBO_ROOT$/tsconfig.json"]`: the task hashes the
    // package and type-checks the project. It runs none of those files and
    // emits no JavaScript made from them: neither the `include` directory nor
    // the file the `include` names.
    let id = "task:packages/assets#type-check";
    assert_eq!(
        memberships(&set, &root, id),
        [
            m("packages/assets/**", "turbo inputs", true),
            m("packages/assets/src/**/*", "tsconfig include", true),
        ]
    );
    let spec = &task(&set, id).spec;
    assert_eq!(rel(&root, &spec.inputs), [] as [&str; 0]);
    assert_eq!(
        rel(&root, &spec.data_inputs),
        ["packages/assets/tools/upload.ts", "tsconfig.json"]
    );
    let part = &spec.parts[0];
    assert_eq!(
        (rel(&root, &part.inputs), rel(&root, &part.data_inputs)),
        (vec![], vec!["packages/assets/tools/upload.ts".to_owned()])
    );

    // `tsc -p` emitting JavaScript: importers of `dist/index.js` run a copy of the
    // sources.
    assert_eq!(
        memberships(&set, &root, "task:packages/assets#build"),
        [m("packages/assets/src/**/*", "tsconfig include", false)]
    );
    assert_eq!(
        memberships(&set, &root, "task:packages/lib#build"),
        [m("packages/lib/src/**/*", "tsconfig include", false)]
    );
    // An unknown command may run or compile any file of its package: nothing of it
    // is read-only.
    let tool = memberships(&set, &root, "task:packages/tool#build");
    assert!(
        !tool.is_empty()
            && tool
                .iter()
                .all(|(p, _, data)| p == "packages/tool/**" && !data),
        "{tool:?}"
    );
}

/// The emit options decide: `--noEmit` on the command line makes any project a
/// type-check; a project that emits JavaScript keeps its named files as plain
/// inputs.
#[test]
fn tsc_emit_options_decide_what_is_only_read() {
    use turborepo_trace_tasks::registry::{RecognizeCx, registry};

    let root = fixture();
    let env = std::collections::BTreeMap::new();
    let recognize = |pkg: &str, line: &str| -> TaskSpec {
        let pkg = root.join(pkg);
        let parsed = turborepo_trace_tasks::shell::parse(line, &pkg, &env);
        let cx = RecognizeCx {
            repo_root: &root,
            package_root: &pkg,
            env: &env,
            task_name: None,
            scripts: &[],
        };
        let mut spec = TaskSpec::default();
        for c in &parsed.commands {
            registry().recognize(c, &cx, &mut spec);
        }
        spec
    };
    let read_only = |spec: &TaskSpec| -> Vec<bool> {
        spec.memberships
            .iter()
            .filter(|m| m.reason == "tsconfig include")
            .map(|m| m.data)
            .collect()
    };
    // `packages/lib` emits JavaScript by its config.
    assert_eq!(
        read_only(&recognize("packages/lib", "tsc -p tsconfig.json")),
        [false]
    );
    assert_eq!(read_only(&recognize("packages/lib", "tsc")), [false]);
    // Switched off on the command line.
    assert_eq!(
        read_only(&recognize("packages/lib", "tsc --noEmit")),
        [true]
    );
    assert_eq!(
        read_only(&recognize("packages/lib", "tsc -p tsconfig.json --noEmit")),
        [true]
    );
    // Variants that run transformer plugins are not known to only read.
    assert_eq!(
        read_only(&recognize("packages/lib", "tspc --noEmit")),
        [false]
    );

    // The file a project names: read by the declaration-only project, compiled when
    // the same sources are emitted (`--emitDeclarationOnly false` overrides the
    // config: the emit is then not known exactly).
    let spec = recognize("packages/assets", "tsc -p tsconfig.json");
    assert_eq!(
        (rel(&root, &spec.inputs), rel(&root, &spec.data_inputs)),
        (vec![], vec!["packages/assets/tools/upload.ts".to_owned()])
    );
    let spec = recognize(
        "packages/assets",
        "tsc -p tsconfig.json --emitDeclarationOnly false",
    );
    assert_eq!(
        (rel(&root, &spec.inputs), rel(&root, &spec.data_inputs)),
        (vec!["packages/assets/tools/upload.ts".to_owned()], vec![])
    );
    assert_eq!(read_only(&spec), [false]);
}

/// `(path[/pattern], kind, owned by a part)` of every output of task `id`.
fn outputs(set: &TaskSet, root: &Path, id: &str) -> Vec<(String, OutputKind, bool)> {
    let mut out: Vec<_> = task(set, id)
        .spec
        .outputs
        .iter()
        .map(|o| {
            let p = o.path.strip_prefix(root).unwrap().to_string_lossy();
            let p = match &o.pattern {
                Some(pat) => format!("{p}/{pat}"),
                None => p.into_owned(),
            };
            (p, o.kind, o.part.is_some())
        })
        .collect();
    out.sort_by(|a, b| (&a.0, a.2).cmp(&(&b.0, b.2)));
    out
}

/// A task made only of `tsc` project parts produces what its parts write. Its
/// declared outputs are a cache list, and its own projects' outputs link to the
/// task as well only when a task of the same package may feed or rewrite them.
/// (Linked to the task, an importer of `dist/index.js` inherited the task's
/// whole dependency chain: on a large internal monorepo, every upstream build
/// script reached through turbo `^build`.)
#[test]
fn a_task_of_tsc_parts_produces_what_its_parts_write() {
    use OutputKind::{Artifact, CacheOnly, Declared};
    let root = fixture();
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let o = |p: &str, kind, part| (p.to_owned(), kind, part);

    // `tsc -b` type-check, turbo `outputs: ["node_modules/.cache/tsbuildinfo.json",
    // "dist"]` and `dependsOn: ["typegen", "^build", "^type-check"]`; the
    // package has no `typegen`.
    assert_eq!(
        outputs(&set, &root, "task:packages/assets#type-check"),
        [
            o("packages/assets/dist", CacheOnly, false),
            o("packages/assets/dist/**/*.d.*ts", Artifact, true),
            o("packages/assets/dist/**/*.d.*ts.map", Artifact, true),
            o("packages/assets/dist/tsbuildinfo.json", Artifact, true),
            o(
                "packages/assets/node_modules/.cache/tsbuildinfo.json",
                CacheOnly,
                false
            ),
        ]
    );
    // `tsc --build --force`, turbo `outputs: ["dist/**"]`, `dependsOn: ["^build"]`
    // with an upstream package: the JavaScript in `dist` is the project's.
    assert_eq!(
        outputs(&set, &root, "task:packages/analytics#build"),
        [
            o("packages/analytics/dist", Artifact, true),
            o("packages/analytics/dist/**", CacheOnly, false),
            o("packages/analytics/dist/tsbuildinfo.json", Artifact, true),
        ]
    );
    // `pnpm typegen && tsc -b` with a `typegen` script in the package: it may
    // generate the project's sources or rewrite its output, so the task stays a
    // producer.
    assert_eq!(
        outputs(&set, &root, "task:packages/typed#build"),
        [
            o("packages/typed/dist", Artifact, false),
            o("packages/typed/dist", Artifact, true),
            o("packages/typed/dist/**", Declared, false),
            o("packages/typed/dist/tsbuildinfo.json", Artifact, false),
            o("packages/typed/dist/tsbuildinfo.json", Artifact, true),
        ]
    );
}
