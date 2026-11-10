//! Output classification ([`OutputKind`]): which task outputs take part in
//! generated-file linking. Fixture: `fixtures/tasks/outputs/`, with monorepo
//! A's `cron-update-turbo` workflow job that made every change select every
//! test (its `mv … package.json` made the root manifest an output of a job that
//! depends on the whole repo).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

use turborepo_trace_tasks::{
    Options, Task, TaskSet, discover,
    sources::Workspace,
    spec::{OutputKind, TaskId},
};

const CRON: &str = "gha:.github/workflows/cron-update-turbo.yml#update-turbo";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/outputs")
        .canonicalize()
        .expect("fixture dir")
}

fn set() -> (PathBuf, TaskSet) {
    let root = fixture();
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    (root, set)
}

fn task<'a>(set: &'a TaskSet, id: &str) -> &'a Task {
    set.get(&id.parse::<TaskId>().unwrap())
        .unwrap_or_else(|| panic!("no task {id}"))
}

/// `(path relative to root, kind)` of every output of `id`.
fn outputs(set: &TaskSet, root: &Path, id: &str) -> Vec<(String, OutputKind)> {
    task(set, id)
        .spec
        .outputs
        .iter()
        .map(|o| {
            let p = o.path.strip_prefix(root).unwrap().to_string_lossy();
            let p = match &o.pattern {
                Some(pat) => format!("{p}/{pat}"),
                None => p.into_owned(),
            };
            (p, o.kind)
        })
        .collect()
}

#[test]
fn snapshot() {
    let (root, set) = set();
    let mut out = String::new();
    for t in &set.tasks {
        if t.spec.outputs.is_empty() {
            continue;
        }
        writeln!(out, "{}", t.id).unwrap();
        for o in &t.spec.outputs {
            let p = o.path.strip_prefix(&root).unwrap().to_string_lossy();
            let p = match &o.pattern {
                Some(pat) => format!("{p}/{pat}"),
                None => p.into_owned(),
            };
            let link = if o.links_producer() {
                "links"
            } else {
                "no link"
            };
            writeln!(out, "  {p}  {}  ({link})", o.kind.as_str()).unwrap();
        }
    }
    insta::assert_snapshot!("outputs", out);
}

#[test]
fn ci_job_is_never_a_producer() {
    let (root, set) = set();
    let job = task(&set, CRON);
    // The job still depends on the whole repo (`jq` is unknown): a valid `--tasks`
    // entry.
    assert!(
        job.spec
            .memberships
            .iter()
            .any(|m| m.dir == root && m.pattern == "**"),
        "{:?}",
        job.spec.memberships
    );
    assert_eq!(
        outputs(&set, &root, CRON),
        [
            ("package.json".to_string(), OutputKind::CiSideEffect),
            ("package.json.tmp".to_string(), OutputKind::CiSideEffect),
        ]
    );
    assert!(job.spec.outputs.iter().all(|o| !o.links_producer()));
    // Nothing produces the root manifest, so tests reaching it do not reach every
    // file.
    let producers: Vec<String> = set
        .producers_of(&root.join("package.json"))
        .iter()
        .map(|t| t.id.to_string())
        .collect();
    assert!(producers.is_empty(), "{producers:?}");
}

#[test]
fn in_place_edits_do_not_link() {
    let (root, set) = set();
    // The same steps in a package script: the existing manifest is edited in place,
    // the temporary file is an artifact.
    assert_eq!(
        outputs(&set, &root, "task:.#bump"),
        [
            ("package.json".to_string(), OutputKind::InPlace),
            ("package.json.tmp".to_string(), OutputKind::Artifact),
        ]
    );
    // A formatter rewrites its files in place.
    assert_eq!(
        outputs(&set, &root, "task:.#format"),
        [("src".to_string(), OutputKind::InPlace)]
    );
    assert!(set.producers_of(&root.join("src/index.ts")).is_empty());
    // A copy into a build dir is an artifact.
    assert_eq!(
        outputs(&set, &root, "task:.#copy-assets"),
        [("dist/logo.svg".to_string(), OutputKind::Artifact)]
    );
}

#[test]
fn generated_and_declared_outputs_link() {
    let (root, set) = set();
    // A code generator's output links, even when committed (`src/generated/api.ts`
    // exists).
    assert_eq!(
        outputs(&set, &root, "task:.#gen"),
        [("src/generated/api.ts".to_string(), OutputKind::Generated)]
    );
    assert_eq!(
        outputs(&set, &root, "task:.#gen-stdout"),
        [(
            "src/generated/api-stdout.ts".to_string(),
            OutputKind::Generated
        )]
    );
    let producers: Vec<String> = set
        .producers_of(&root.join("src/generated/api.ts"))
        .iter()
        .map(|t| t.id.to_string())
        .collect();
    assert_eq!(producers, ["task:.#gen"]);
    // turbo `outputs` are declared.
    assert_eq!(
        outputs(&set, &root, "task:.#build"),
        [("dist/**".to_string(), OutputKind::Declared)]
    );
    assert!(task(&set, "task:.#build").spec.outputs[0].links_producer());
    assert_eq!(
        set.producers_of(&root.join("dist/index.js"))
            .iter()
            .map(|t| t.id.to_string())
            .collect::<Vec<_>>(),
        ["task:.#build"]
    );
}

#[test]
fn docker_build_outputs_are_side_effects() {
    let (root, set) = set();
    assert_eq!(
        outputs(&set, &root, "task:.#image"),
        [("image.id".to_string(), OutputKind::CiSideEffect)]
    );
}

/// Fixture `fixtures/tasks/turbo-scope/`: a test server's build
/// declares `$TURBO_ROOT$/**/dist/**/*` (collecting its dependencies' `dist/`
/// for the cache) and a root task `//#cache-tsbuildinfo-files`
/// declares other packages' `dist/tsbuildinfo.json`. Declared outputs link only
/// inside the declaring package: every `dist/` is produced by its own package's
/// build, and still is (soundness).
#[test]
fn declared_outputs_are_scoped_to_their_package() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/turbo-scope")
        .canonicalize()
        .expect("fixture dir");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let producers = |f: &str| -> Vec<String> {
        set.producers_of(&root.join(f))
            .iter()
            .map(|t| t.id.to_string())
            .collect()
    };
    assert_eq!(
        producers("packages/lib/dist/index.js"),
        ["task:packages/lib#build"]
    );
    assert_eq!(
        producers("packages/lib/dist/tsbuildinfo.json"),
        ["task:packages/lib#build"]
    );
    assert_eq!(
        producers("packages/server/dist/main.js"),
        ["task:packages/server#build"]
    );
    assert_eq!(
        producers("packages/server/src/dist/x.js"),
        ["task:packages/server#build"],
        "the in-package part of the glob still links"
    );
    // The cross-package declarations are recorded, but don't link.
    assert_eq!(
        outputs(&set, &root, "task:packages/server#build"),
        [
            ("/**/dist/**/*".to_string(), OutputKind::CacheOnly),
            (
                "packages/server/**/dist/**/*".to_string(),
                OutputKind::Declared
            ),
            ("packages/server/dist/**".to_string(), OutputKind::Declared),
        ]
    );
    // `echo` only: the task writes nothing, so none of its declared outputs link.
    let cache = outputs(&set, &root, "task:.#cache-tsbuildinfo-files");
    assert!(
        cache
            .iter()
            .any(|(p, k)| p == "packages/*/dist/tsbuildinfo.json" && *k == OutputKind::CacheOnly),
        "{cache:?}"
    );
    assert!(
        cache.iter().all(|(_, k)| *k == OutputKind::CacheOnly),
        "{cache:?}"
    );
    // An `echo` that writes a file is not "no file effect": its declared output
    // links.
    assert_eq!(producers("stamp.txt"), ["task:.#stamp"]);
    // A `dev:ts` task (`tsc -b -w`, turbo `persistent: true`): its outputs
    // are recorded but never link, so `lib/dist` is produced by `build` only.
    let dev = task(&set, "task:packages/lib#dev:ts");
    assert!(dev.spec.persistent);
    assert!(!dev.spec.outputs.is_empty(), "tsc -b writes outputs");
    assert!(
        dev.spec
            .outputs
            .iter()
            .all(|o| o.kind == OutputKind::Persistent && !o.links_producer()),
        "{:?}",
        dev.spec.outputs
    );
}

/// `(path relative to root[/pattern], part relative to root)` of every linking
/// output of `id`.
fn linking_outputs(set: &TaskSet, root: &Path, id: &str) -> Vec<(String, Option<String>)> {
    let rel = |p: &Path| p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
    task(set, id)
        .spec
        .outputs
        .iter()
        .filter(|o| o.links_producer())
        .map(|o| {
            let p = rel(&o.path);
            let p = match &o.pattern {
                Some(pat) => format!("{p}/{pat}"),
                None => p,
            };
            (p, o.part.as_deref().map(rel))
        })
        .collect()
}

/// Fixture `fixtures/tasks/tsc-solution/` (the shape of a large monorepo): a
/// root `tsc -b` over a solution tsconfig, apps whose `type-check` is `tsc -b`
/// with references to the packages they use, and packages consumed through
/// their built `dist/`.
///
/// Every `tsc -b` that reaches a project writes that project's `outDir`.
/// Recorded as outputs of the whole task, an importer of
/// `packages/icons/dist/index.js` depended on every source of every project of
/// every such task (the root solution: the whole repository). The outputs of a
/// project now belong to the project's part: its config chain, its sources and
/// the projects it references.
#[test]
fn tsc_build_outputs_belong_to_their_project() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/tsc-solution")
        .canonicalize()
        .expect("fixture dir");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let part = |p: &str| Some(p.to_string());
    let icons = part("packages/icons/tsconfig.json");
    let patched = part("packages/patched/tsconfig.json");
    let ui = part("packages/ui/tsconfig.json");
    let o = |s: &str| s.to_string();

    // The root solution: every output is owned by its project, none by the task.
    let solution = linking_outputs(&set, &root, "task:.#build:ts-debug");
    assert_eq!(
        solution,
        [
            (
                o("apps/docs/node_modules/.cache/tsbuildinfo.json"),
                part("apps/docs/tsconfig.json")
            ),
            (
                o("apps/web/node_modules/.cache/tsbuildinfo.json"),
                part("apps/web/tsconfig.json")
            ),
            (o("packages/icons/dist"), icons.clone()),
            (o("packages/icons/dist/tsbuildinfo.json"), icons.clone()),
            (o("packages/patched/dist"), patched.clone()),
            (o("packages/patched/dist/tsbuildinfo.json"), patched.clone()),
            // `emitDeclarationOnly` (inherited): declarations, not the `index.js` that
            // runtime importers load. A `noEmit` app (inherited too) emits nothing but its
            // build info.
            (o("packages/ui/dist/**/*.d.*ts"), ui.clone()),
            (o("packages/ui/dist/**/*.d.*ts.map"), ui.clone()),
            (o("packages/ui/dist/tsbuildinfo.json"), ui.clone()),
        ]
    );
    let t = task(&set, "task:.#build:ts-debug");
    // The task itself still depends on everything (a valid `--tasks` entry).
    for dir in [
        "apps/web",
        "apps/docs",
        "packages/icons/src",
        "packages/ui/src",
    ] {
        assert!(
            t.spec.memberships.iter().any(|m| m.dir == root.join(dir)),
            "{dir}: {:?}",
            t.spec.memberships
        );
    }
    // The part of `packages/ui`: its own config chain and sources, and the project
    // it references. Nothing of the apps.
    let p = t
        .spec
        .parts
        .iter()
        .find(|p| p.path == root.join("packages/ui/tsconfig.json"))
        .expect("ui part");
    assert_eq!(p.kind, "tsproject");
    let rel = |p: &Path| {
        p.strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(
        p.configs.iter().map(|c| rel(c)).collect::<Vec<_>>(),
        [
            "packages/ui/tsconfig.json",
            "tooling/tsconfig/bundler.json",
            "tooling/tsconfig/base.json"
        ]
    );
    assert_eq!(
        p.memberships
            .iter()
            .map(|m| format!("{}/{}", rel(&m.dir), m.pattern))
            .collect::<Vec<_>>(),
        ["packages/ui/src/**/*"]
    );
    assert_eq!(
        p.deps.iter().map(|d| rel(d)).collect::<Vec<_>>(),
        ["packages/icons/tsconfig.json"]
    );
    assert!(p.toolchain.iter().any(|t| t.name == "typescript"));

    // An app's `tsc -b` (turbo `dependsOn: ["typegen", "^build", "^type-check"]`):
    // the packages' outputs belong to the packages' projects. So does the app's
    // own project: `web` has no `typegen` script, and the tasks of other
    // packages (`^build`) don't write into this one. Every output belongs to a
    // part; none links to the task, so an importer does not inherit the task's
    // dependency chain.
    let web = linking_outputs(&set, &root, "task:apps/web#type-check");
    assert!(
        web.contains(&(o("packages/icons/dist"), icons.clone())),
        "{web:?}"
    );
    assert!(
        !web.iter()
            .any(|(o, p)| o.starts_with("packages/") && p.is_none()),
        "{web:?}"
    );
    let info = o("apps/web/node_modules/.cache/tsbuildinfo.json");
    assert!(
        web.contains(&(info, part("apps/web/tsconfig.json"))),
        "{web:?}"
    );
    assert!(web.iter().all(|(_, p)| p.is_some()), "{web:?}");
    // `pnpm typegen && tsc -b` in `docs`, which has a `typegen` script: running
    // another task first changes nothing for the packages' projects. The app's
    // own project stays an output of the task as well: a task of the same
    // package may generate its sources or rewrite what it emits.
    let docs = linking_outputs(&set, &root, "task:apps/docs#type-check");
    assert!(
        docs.contains(&(o("packages/ui/dist/**/*.d.*ts"), ui.clone())),
        "{docs:?}"
    );
    assert!(
        docs.contains(&(o("apps/docs/node_modules/.cache/tsbuildinfo.json"), None)),
        "{docs:?}"
    );
    assert!(
        !docs
            .iter()
            .any(|(o, p)| o.starts_with("packages/") && p.is_none()),
        "{docs:?}"
    );

    // Soundness. `tsc -p` with the package's own build config is that package's
    // task.
    let build = linking_outputs(&set, &root, "task:packages/icons#build");
    assert!(
        build.contains(&(o("packages/icons/dist"), None)),
        "{build:?}"
    );
    assert!(
        task(&set, "task:packages/icons#build")
            .spec
            .parts
            .is_empty(),
        "tsc -p has no parts"
    );
    // The `build` config of `ui` turns `emitDeclarationOnly` off: it emits the
    // JavaScript.
    assert!(
        linking_outputs(&set, &root, "task:packages/ui#build")
            .contains(&(o("packages/ui/dist"), None)),
    );
    // `tsc -b && node scripts/patch-dist.js`: the script may rewrite what `tsc`
    // wrote for the package's own project, so those outputs stay outputs of the
    // whole task (which depends on the script). The project it references in
    // another package is not its to rewrite: `icons/dist` belongs to the
    // `icons` project only.
    let patch = task(&set, "task:packages/patched#build");
    let outs = linking_outputs(&set, &root, "task:packages/patched#build");
    assert!(
        outs.contains(&(o("packages/patched/dist"), None)),
        "{outs:?}"
    );
    assert!(
        outs.contains(&(o("packages/icons/dist"), icons.clone())),
        "{outs:?}"
    );
    assert!(
        !outs
            .iter()
            .any(|(o, p)| o.starts_with("packages/icons/") && p.is_none()),
        "{outs:?}"
    );
    assert!(
        patch
            .spec
            .entries
            .iter()
            .any(|e| e.path == root.join("packages/patched/scripts/patch-dist.js")),
        "{:?}",
        patch.spec.entries
    );
}

/// A root script may write anywhere in the repository: with a command whose
/// writes are unknown next to `tsc -b` (`node scripts/gen.js && tsc -b`), every
/// output stays an output of the whole task, also those of projects in
/// workspace packages.
#[test]
fn opaque_root_task_keeps_its_outputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/tsc-root-opaque")
        .canonicalize()
        .expect("fixture dir");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let t = task(&set, "task:.#build");
    assert!(t.spec.parts.is_empty(), "{:?}", t.spec.parts);
    assert_eq!(
        linking_outputs(&set, &root, "task:.#build"),
        [("packages/lib/dist".to_string(), None)]
    );
    // The same solution built by a transparent task has its parts.
    assert_eq!(
        linking_outputs(&set, &root, "task:.#type-check"),
        [(
            "packages/lib/dist".to_string(),
            Some("packages/lib/tsconfig.json".to_string())
        )]
    );
}
