//! Services a task starts before the tests it runs
//! (`crates/turborepo-trace/src/services.rs`).
//!
//! The shape comes from a real monorepo: a Go service with TypeScript
//! integration tests that reach it over HTTP. The CI job runs `docker compose
//! up --wait` in the service's directory (the compose file builds the service
//! from its Dockerfile and mounts config files into third-party containers),
//! then `pnpm integration`. No import leads from the tests to the Go code, the
//! Dockerfile, the compose file or the mounted files.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

const WORKFLOW: &str = "\
name: Integration
on: [pull_request]
jobs:
  ingest-integration:
    name: Integration tests - ingest
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Start service and dependencies
        run: |
          cd services/ingest
          docker compose up --wait
      - name: Run integration tests
        run: |
          cd services/ingest
          pnpm integration
  other-e2e:
    name: E2E tests - other
    runs-on: ubuntu-latest
    env:
      COMPOSE_FILE: 'packages/other/docker-compose.e2e.yml'
    steps:
      - uses: actions/checkout@v4
      - name: Start docker compose
        run: |
          docker compose version
          docker compose up -d
      - name: Run tests against docker compose
        run: pnpm --filter=@fx/other run test
";

const COMPOSE: &str = "\
services:
  store:
    image: example/store:25
    volumes:
      - ./integration/config/store-init.sql:/docker-entrypoint-initdb.d/init.sql
      - store-data:/var/lib/store
  mock:
    image: example/mockserver:5
    volumes:
      - type: bind
        source: ./integration/config/expectations.json
        target: /config/expectations.json
    env_file: integration/config/mock.env
  ingest:
    build:
      context: ../..
      dockerfile: services/ingest/Dockerfile
    depends_on: [store, mock]
    ports: ['8888:8888']
volumes:
  store-data:
";

const DOCKERFILE: &str = "\
FROM golang:1.24-alpine AS builder
WORKDIR /build
COPY services/ingest/app services/ingest/app
COPY pkg/schema pkg/schema
RUN go build -o server ./services/ingest/app

FROM alpine AS runtime
COPY --from=builder /build/server .
CMD [\"./server\"]
";

/// The repository: the Go service with its integration tests and CI job, a
/// package whose script starts its own compose project before its tests, and an
/// unrelated package.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, ".gitignore", "node_modules\n.turbo/trace\n");
    // The excludes of a `.dockerignore` end up in the name of every COPY
    // membership.
    write(r, ".dockerignore", "**/node_modules\n.git/\n");
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'services/*'\n  - 'packages/*'\n",
    );
    write(r, ".github/workflows/integration.yml", WORKFLOW);
    // The Go service and its TypeScript integration tests.
    write(
        r,
        "services/ingest/package.json",
        r#"{"name":"ingest","version":"1.0.0","scripts":{"test":"cd app && go test ./...","integration":"jest"}}"#,
    );
    write(
        r,
        "services/ingest/jest.config.js",
        "module.exports = {};\n",
    );
    write(r, "services/ingest/docker-compose.yml", COMPOSE);
    write(r, "services/ingest/Dockerfile", DOCKERFILE);
    write(r, "services/ingest/README.md", "# ingest\n");
    write(
        r,
        "services/ingest/app/go.mod",
        "module example.com/ingest\n\ngo 1.24\n",
    );
    write(
        r,
        "services/ingest/app/main.go",
        "package main\n\nfunc main() {}\n",
    );
    write(
        r,
        "services/ingest/app/handler/handler.go",
        "package handler\n\nfunc Handle() int { return 200 }\n",
    );
    write(
        r,
        "pkg/schema/schema.go",
        "package schema\n\nconst Version = 1\n",
    );
    write(
        r,
        "services/ingest/integration/config/store-init.sql",
        "CREATE TABLE events (id String) ENGINE = Memory;\n",
    );
    write(
        r,
        "services/ingest/integration/config/expectations.json",
        "[]\n",
    );
    write(
        r,
        "services/ingest/integration/config/mock.env",
        "MOCK_PORT=1080\n",
    );
    write(
        r,
        "services/ingest/integration/utils/urls.ts",
        "export const URL_INGEST = process.env.URL_INGEST ?? 'http://localhost:8888';\n",
    );
    write(
        r,
        "services/ingest/integration/events.test.ts",
        "import { URL_INGEST } from './utils/urls';\n\ntest('accepts events', async () => {\n  \
         const res = await fetch(`${URL_INGEST}/v1/events`, { method: 'POST', body: '{}' });\n  \
         expect(res.status).toBe(200);\n});\n",
    );
    // A package script that starts its own project through another script, then
    // tests.
    write(
        r,
        "packages/registry/package.json",
        r#"{"name":"@fx/registry","version":"1.0.0","scripts":{"services:up":"docker compose up -d","test:integration":"pnpm services:up && jest"}}"#,
    );
    write(
        r,
        "packages/registry/jest.config.js",
        "module.exports = {};\n",
    );
    write(
        r,
        "packages/registry/docker-compose.yml",
        "services:\n  db:\n    image: example/db-local:2\n    volumes:\n      - \
         ./seed/tables.json:/seed/tables.json:ro\n",
    );
    write(r, "packages/registry/seed/tables.json", "{}\n");
    write(
        r,
        "packages/registry/src/client.js",
        "module.exports = 1;\n",
    );
    write(
        r,
        "packages/registry/src/client.test.js",
        "const c = require('./client');\n\ntest('client', () => {\n  expect(c).toBe(1);\n});\n",
    );
    // A package whose CI job names its compose file through `COMPOSE_FILE` and
    // starts it from the repository root; the root has a compose file of its
    // own.
    write(
        r,
        "docker-compose.yml",
        "services:\n  cache:\n    image: example/cache:7\n",
    );
    write(
        r,
        "packages/other/docker-compose.e2e.yml",
        "services:\n  queue:\n    image: example/queue:1\n    volumes:\n      - \
         './e2e/queues.conf:/etc/queues.conf'\n",
    );
    write(r, "packages/other/e2e/queues.conf", "queues {}\n");
    write(
        r,
        "packages/other/package.json",
        r#"{"name":"@fx/other","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "packages/other/jest.config.js", "module.exports = {};\n");
    write(r, "packages/other/src/o.js", "module.exports = 1;\n");
    write(
        r,
        "packages/other/src/o.test.js",
        "const o = require('./o');\n\ntest('o', () => {\n  expect(o).toBe(1);\n});\n",
    );
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);
    d
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

/// The jest tests a change of `path` selects through real paths (uncertainty
/// ignored).
fn selected(root: &Path, path: &str) -> Vec<String> {
    turbotrace(
        root,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            path,
            "--on-uncertain",
            "ignore",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

const INGEST: &[&str] = &["services/ingest/integration/events.test.ts"];
const REGISTRY: &[&str] = &["packages/registry/src/client.test.js"];

#[test]
fn a_job_that_starts_a_compose_project_runs_its_tests_against_it() {
    let d = repo();
    let r = d.path();
    // What the service is built from (the COPY sources of its Dockerfile).
    assert_eq!(selected(r, "services/ingest/app/main.go"), INGEST);
    assert_eq!(
        selected(r, "services/ingest/app/handler/handler.go"),
        INGEST
    );
    assert_eq!(selected(r, "pkg/schema/schema.go"), INGEST);
    // An added source file.
    write(r, "services/ingest/app/new.go", "package main\n");
    assert_eq!(selected(r, "services/ingest/app/new.go"), INGEST);
    // The build and the project themselves.
    assert_eq!(selected(r, "services/ingest/Dockerfile"), INGEST);
    assert_eq!(selected(r, "services/ingest/docker-compose.yml"), INGEST);
    // Files mounted into third-party containers, in both volume syntaxes, and an
    // env file.
    assert_eq!(
        selected(r, "services/ingest/integration/config/store-init.sql"),
        INGEST
    );
    assert_eq!(
        selected(r, "services/ingest/integration/config/expectations.json"),
        INGEST
    );
    assert_eq!(
        selected(r, "services/ingest/integration/config/mock.env"),
        INGEST
    );
}

#[test]
fn a_script_that_starts_a_project_through_another_script() {
    let d = repo();
    let r = d.path();
    // `test:integration` runs `services:up` (which runs no tests) and then jest.
    assert_eq!(selected(r, "packages/registry/seed/tables.json"), REGISTRY);
    assert_eq!(
        selected(r, "packages/registry/docker-compose.yml"),
        REGISTRY
    );
}

#[test]
fn a_job_names_its_compose_file_in_the_environment() {
    let d = repo();
    let r = d.path();
    let other: &[&str] = &["packages/other/src/o.test.js"];
    // `COMPOSE_FILE` of the job, not the compose file of the directory the step
    // runs in.
    assert_eq!(selected(r, "packages/other/e2e/queues.conf"), other);
    assert_eq!(selected(r, "packages/other/docker-compose.e2e.yml"), other);
    // The root project is started by nothing that runs tests.
    assert!(selected(r, "docker-compose.yml").is_empty());
}

#[test]
fn files_outside_the_services_select_nothing_through_them() {
    let d = repo();
    let r = d.path();
    // Not copied into the image, not mounted.
    assert!(selected(r, "services/ingest/README.md").is_empty());
    // Another package: its own test through the import, nothing of the services.
    assert_eq!(
        selected(r, "packages/other/src/o.js"),
        ["packages/other/src/o.test.js"]
    );
    // A test file is its own change only.
    assert_eq!(
        selected(r, "services/ingest/integration/utils/urls.ts"),
        INGEST
    );
    assert_eq!(selected(r, "packages/registry/src/client.js"), REGISTRY);
}

#[test]
fn the_reason_names_the_service_and_the_task() {
    let d = repo();
    let r = d.path();
    let why = turbotrace(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "services/ingest/app/main.go",
            "--on-uncertain",
            "ignore",
            "--explain",
            "services/ingest/integration/events.test.ts",
        ],
    );
    assert!(
        why.contains("affected services/ingest/integration/events.test.ts")
            && why.contains("services/ingest/app/main.go")
            && why.contains("docker:services/ingest/docker-compose.yml")
            && why.contains("gha:.github/workflows/integration.yml#ingest-integration"),
        "{why}"
    );
}
