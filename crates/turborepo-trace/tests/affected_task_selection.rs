//! `turbotrace affected --tasks`: which build tasks a small edit selects in a
//! workspace whose root package has scripts turbotrace cannot read (a service
//! image build, a notification hook) next to a repository-wide lint config.
//!
//! The shape is the one of a large service monorepo: one root shell script
//! builds the image of the service named by an environment variable, a root
//! config script lists the services to configure a linter that every service
//! runs, and CI jobs call tools the task model does not know.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn turbotrace(root: &Path, args: &[&str]) -> String {
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// The tasks a one-file edit selects, with the uncertainty floor off: what the
/// change itself reaches.
fn tasks(root: &Path, changed: &str) -> Vec<String> {
    turbotrace(
        root,
        &[
            "affected",
            "--tasks",
            "**",
            "--changed",
            changed,
            "--on-uncertain",
            "ignore",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

const BUILD_IMAGE: &str = r#"#!/usr/bin/env bash
set -euo pipefail
SERVICE_DIR="services/$SERVICE"
BUILD_DIR="."
if jq -e '.["ci-cd"].prune == true' "$SERVICE_DIR/package.json" > /dev/null; then
  BUILD_DIR=".pruned-$SERVICE"
  turbo prune "$SERVICE" --out-dir "$BUILD_DIR"
  sed 's/^  - tools$//' "$BUILD_DIR/pnpm-workspace.yaml" > "$BUILD_DIR/pnpm-workspace.yaml.tmp" && \
    mv "$BUILD_DIR/pnpm-workspace.yaml.tmp" "$BUILD_DIR/pnpm-workspace.yaml"
fi
docker build -t "$TAG" --build-arg "BUILD_DIR=$BUILD_DIR" -f ./Dockerfile-service "$BUILD_DIR"
docker run --rm "$TAG" cat /tmp/metrics.txt > /tmp/build-metrics.txt
"#;

/// The linter config every service names. It looks for nested workspaces by a
/// computed path, which makes it a reader of a repository-wide listing.
const KNIP_CONFIG: &str = r#"const { existsSync, readFileSync } = require('node:fs');
const { join } = require('node:path');

// Every top-level group named in KNIP_GROUPS that is a workspace of its own.
const groups = (process.env.KNIP_GROUPS ?? 'services').split(',');
const workspaces = {};
for (const group of groups) {
  const file = join(__dirname, group, 'pnpm-workspace.yaml');
  if (!existsSync(file)) continue;
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    const m = /^\s*-\s*'?([\w-]+)'?$/.exec(line);
    if (m) workspaces[`${group}/${m[1]}`] = { entry: ['src/index.ts'] };
  }
}
// The service list is kept in a module; only its text is read here.
const listed = readFileSync(join(__dirname, 'scripts/services.mjs'), 'utf8');
module.exports = { workspaces, ignore: listed.includes('legacy') ? ['legacy/**'] : [] };
"#;

/// A module that the linter config reads as text. Run, it loads a plugin named
/// by the environment (an open module load).
const SERVICES: &str = r#"import { prefix } from './lib/names.mjs';

export const services = ['svc-a', 'web'].map((s) => prefix + s);
export const plugin = await import(process.env.SERVICE_PLUGIN);
"#;

/// A service's build: copies the source into `dist/` (declared in
/// `turbo.json`).
const BUILD: &str = r#"import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const source = readFileSync(new URL('./src/index.ts', import.meta.url), 'utf8');
mkdirSync(new URL('./dist/', import.meta.url), { recursive: true });
writeFileSync(new URL('./dist/bundle.js', import.meta.url), source);
"#;

/// A root script that reads one service's build output.
const SIZE: &str = r#"import { readdirSync } from 'node:fs';

console.log(readdirSync(new URL('../services/svc-a/dist', import.meta.url)).length);
"#;

fn service(r: &Path, name: &str) {
    let dir = format!("services/{name}");
    write(
        r,
        &format!("{dir}/package.json"),
        &format!(
            r#"{{
  "name": "@acme/{name}",
  "main": "src/index.ts",
  "scripts": {{
    "build": "node build.mjs",
    "clean-code": "knip --config ../../knip.config.js --no-config-hints",
    "clean-code:fix": "pnpm clean-code --fix",
    "ci:build": "TAG={name} SERVICE={name} pnpm -w build-image",
    "dev": "nodemon src/index.ts",
    "test": "jest"
  }}
}}
"#
        ),
    );
    write(r, &format!("{dir}/build.mjs"), BUILD);
    write(
        r,
        &format!("{dir}/jest.config.js"),
        "module.exports = {};\n",
    );
    write(
        r,
        &format!("{dir}/src/index.ts"),
        "export const answer = 42;\n",
    );
    write(
        r,
        &format!("{dir}/src/index.test.ts"),
        "import { answer } from './index';\ntest('answer', () => expect(answer).toBe(42));\n",
    );
}

/// The workspace: two services, a root package with an image build script, a
/// hook script and a script over named workspaces, and the shared linter
/// config.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{
  "name": "acme",
  "private": true,
  "scripts": {
    "build-image": "./build-image.sh",
    "notify": "curl -fsS https://hooks.example.test/deployed",
    "scan": "sg scan services",
    "services": "node scripts/services.mjs",
    "size": "node scripts/size.mjs"
  }
}
"#,
    );
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'services/*'\n");
    write(r, "build-image.sh", BUILD_IMAGE);
    write(r, "Dockerfile-service", "FROM node:22\nCOPY . .\n");
    write(r, "knip.config.js", KNIP_CONFIG);
    write(
        r,
        "turbo.json",
        r#"{ "tasks": { "build": { "outputs": ["dist/**"] } } }
"#,
    );
    write(r, "scripts/services.mjs", SERVICES);
    write(
        r,
        "scripts/lib/names.mjs",
        "export const prefix = '@acme/';\n",
    );
    write(r, "scripts/size.mjs", SIZE);
    write(r, "services/README.md", "# Services\n");
    write(r, "docs/overview.md", "# Overview\n");
    write(r, "tools/README.md", "# Tools\n");
    service(r, "svc-a");
    service(r, "web");
    d
}

fn has(list: &[String], id: &str) -> bool {
    list.iter().any(|t| t == id)
}

/// The tasks a one-file edit selects under the default policy.
fn tasks_by_default(root: &Path, changed: &str) -> Vec<String> {
    turbotrace(root, &["affected", "--tasks", "**", "--changed", changed])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The linter config lists the repository root with a computed path (`**`).
/// Each service's build declares `dist/**`, which such a listing would see if
/// it were there: the config was a consumer of every build, and a source edit
/// in one service selected the lint task of every other. A reader that names
/// the output (the size script lists `services/svc-a/dist`) is a consumer.
#[test]
fn a_wildcard_listing_does_not_make_its_reader_a_consumer_of_every_build() {
    let d = repo();
    let r = d.path();
    let sel = tasks(r, "services/svc-a/src/index.ts");
    for id in [
        "task:services/svc-a#build",
        "task:services/svc-a#clean-code",
        "task:services/svc-a#test",
        "task:.#size",
    ] {
        assert!(has(&sel, id), "{id}: {sel:?}");
    }
    for id in [
        "task:services/web#build",
        "task:services/web#clean-code",
        "task:services/web#test",
    ] {
        assert!(!has(&sel, id), "{id}: {sel:?}");
    }
    // The other service's source does not reach the size script.
    let sel = tasks(r, "services/web/src/index.ts");
    assert!(has(&sel, "task:services/web#build"), "{sel:?}");
    assert!(!has(&sel, "task:.#size"), "{sel:?}");
}

/// The linter config reads `scripts/services.mjs` as text. The lint tasks
/// follow that file's own change, not what the module imports, and its open
/// module load is not theirs: nothing runs the module when the linter runs. The
/// script that runs it has both.
#[test]
fn a_file_read_as_bytes_carries_its_own_change_only() {
    let d = repo();
    let r = d.path();
    let lint = [
        "task:services/svc-a#clean-code",
        "task:services/web#clean-code",
    ];
    let sel = tasks(r, "scripts/services.mjs");
    for id in lint.iter().chain(&["task:.#services"]) {
        assert!(has(&sel, id), "{id}: {sel:?}");
    }
    let sel = tasks(r, "scripts/lib/names.mjs");
    assert!(has(&sel, "task:.#services"), "{sel:?}");
    for id in lint {
        assert!(!has(&sel, id), "{id}: {sel:?}");
    }
    // The floor: an unrelated edit under the default policy.
    let sel = tasks_by_default(r, "docs/overview.md");
    assert!(has(&sel, "task:.#services"), "{sel:?}");
    for id in lint {
        assert!(!has(&sel, id), "{id}: {sel:?}");
    }
}

/// A wildcard reports what has to be built, checked and tested again: not the
/// dev server and not the `--fix` variant of a check, which depend on the same
/// files. `--all-tasks`, an exact id, or a CI job that runs the task bring it
/// back.
#[test]
fn a_wildcard_leaves_out_tasks_run_by_hand() {
    let d = repo();
    let r = d.path();
    let changed = "services/svc-a/src/index.ts";
    let with = |extra: &[&str]| -> Vec<String> {
        let mut args = vec!["affected", "--changed", changed, "--on-uncertain", "ignore"];
        args.extend_from_slice(extra);
        turbotrace(r, &args).lines().map(str::to_owned).collect()
    };
    let fix = "task:services/svc-a#clean-code:fix";
    let dev = "task:services/svc-a#dev";
    let sel = with(&["--tasks", "**"]);
    assert!(has(&sel, "task:services/svc-a#clean-code"), "{sel:?}");
    assert!(!has(&sel, fix) && !has(&sel, dev), "{sel:?}");
    let sel = with(&["--tasks", "**", "--all-tasks"]);
    assert!(has(&sel, fix) && has(&sel, dev), "{sel:?}");
    let sel = with(&["--tasks", "services/svc-a#clean-code:fix"]);
    assert_eq!(sel, [fix]);
    // A workflow step that runs the fix variant: part of the automation.
    write(
        r,
        ".github/workflows/fix.yml",
        "name: fix\non: push\njobs:\n  fix:\n    runs-on: ubuntu-latest\n    steps:\n      - \
         uses: actions/checkout@v4\n      - run: pnpm --filter @acme/svc-a clean-code:fix\n",
    );
    let sel = with(&["--tasks", "**"]);
    assert!(has(&sel, fix), "{sel:?}");
    assert!(!has(&sel, dev), "{sel:?}");
}
