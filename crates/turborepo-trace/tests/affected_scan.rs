//! A script that scans a source tree, and a test that compares its result with
//! a committed snapshot.
//!
//! The shape comes from a real monorepo: an app's script walks `<app>/src` with
//! a recursive `readdirSync` (the files are collected in an accumulator
//! parameter), reads every source file it finds and derives a registry from
//! them; a jest test builds the registry and compares it with a committed JSON
//! file. Adding a page without regenerating the file fails the test ("the
//! registry is stale"). No import leads from the test to the pages:
//! the dependency is the scanned tree.
//!
//! The value-flow engine does not follow the accumulator, so the read of the
//! walked files has no path of its own. What selects the test is the bound on
//! such reads: the names the script's text gives (it joins `'src'` onto its
//! root, and its root defaults to the working directory). The first test pins
//! that a change inside the scanned tree selects the test, with and without the
//! uncertainty policy: whatever closes or bounds that read must keep it. The
//! second is the goal and is ignored: the scan as a dependency on exactly
//! the listed tree, so that a `src` directory of another app does not select
//! the test.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const SCRIPT: &str = r#"import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const OUTPUT_FILE = 'registry.json';
const SOURCE_EXTENSIONS = ['.ts', '.tsx', '.js', '.jsx'];

function walkSourceFiles(directory: string, files: string[] = []): string[] {
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort(
    (a, b) => a.name.localeCompare(b.name),
  )) {
    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name === '__tests__') {
        continue;
      }
      walkSourceFiles(absolutePath, files);
      continue;
    }

    if (
      entry.isFile() &&
      SOURCE_EXTENSIONS.includes(path.extname(entry.name)) &&
      !entry.name.endsWith('.d.ts')
    ) {
      files.push(absolutePath);
    }
  }

  return files;
}

function indexSourceFiles(
  sourceFiles: string[],
  appSource: string,
): Map<string, string> {
  const fileInfo = new Map<string, string>();

  for (const file of sourceFiles) {
    const sourceText = readFileSync(file, 'utf8');
    fileInfo.set(path.relative(appSource, file), sourceText);
  }

  return fileInfo;
}

function staticRewrites(appRoot: string): string {
  const configPath = SOURCE_EXTENSIONS.map((extension) =>
    path.join(appRoot, `app.config${extension}`),
  ).find(existsSync);
  if (!configPath) {
    return '';
  }

  const sourceText = readFileSync(configPath, 'utf8');
  return sourceText;
}

export function buildRegistry(appRoot: string = process.cwd()): string[] {
  const appSource = path.join(appRoot, 'src');
  const sourceFiles = walkSourceFiles(appSource);
  const fileInfo = indexSourceFiles(sourceFiles, appSource);
  const rewrites = staticRewrites(appRoot);
  return [...fileInfo.keys(), rewrites].sort();
}

export function writeRegistry(appRoot: string = process.cwd()): string {
  const registry = buildRegistry(appRoot);
  const outputPath = path.join(appRoot, OUTPUT_FILE);
  writeFileSync(outputPath, `${JSON.stringify(registry, null, 2)}\n`);
  return outputPath;
}
"#;

const TEST: &str = r#"import { readFileSync } from 'node:fs';
import path from 'node:path';
import { buildRegistry } from './snapshot-registry';

const appRoot = path.resolve(__dirname, '../..');

describe('registry', () => {
  const registry = buildRegistry(appRoot);

  it('matches the committed snapshot', () => {
    const committed = JSON.parse(
      readFileSync(path.join(appRoot, 'registry.json'), 'utf8'),
    );
    expect(registry).toEqual(committed);
  });
});
"#;

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n");
    write(
        r,
        "apps/admin/package.json",
        r#"{"name":"admin","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/admin/jest.config.js", "module.exports = {};\n");
    write(r, "apps/admin/app.config.ts", "export default {};\n");
    write(
        r,
        "apps/admin/registry.json",
        "[\"\", \"app/page.tsx\", \"lib/client.ts\"]\n",
    );
    write(
        r,
        "apps/admin/src/app/page.tsx",
        "export default function Page() { return null; }\n",
    );
    write(
        r,
        "apps/admin/src/lib/client.ts",
        "export const get = () => 1;\n",
    );
    write(
        r,
        "apps/admin/scripts/registry/snapshot-registry.ts",
        SCRIPT,
    );
    write(
        r,
        "apps/admin/scripts/registry/snapshot-registry.test.ts",
        TEST,
    );
    // Another app, with a test of its own.
    write(
        r,
        "apps/other/package.json",
        r#"{"name":"other","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/other/jest.config.js", "module.exports = {};\n");
    write(r, "apps/other/src/o.js", "module.exports = 1;\n");
    write(
        r,
        "apps/other/src/o.test.js",
        "const o = require('./o');\n\ntest('o', () => {\n  expect(o).toBe(1);\n});\n",
    );
    d
}

fn selected(root: &Path, path: &str, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest", "--changed", path];
    args.extend_from_slice(extra);
    let o = Command::new(support::harness())
        .args(&args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

const SNAPSHOT_TEST: &str = "apps/admin/scripts/registry/snapshot-registry.test.ts";

/// The snapshot test is selected when a scanned file changes or is added, when
/// the config the script reads changes, and when the snapshot changes: under
/// the default policy, and for the scanned files also when uncertainty is
/// ignored (a real path).
#[test]
fn the_snapshot_test_is_selected_for_changes_in_the_scanned_tree() {
    let d = repo();
    let r = d.path();
    for path in [
        "apps/admin/src/app/page.tsx",
        "apps/admin/src/lib/client.ts",
        "apps/admin/app.config.ts",
        "apps/admin/registry.json",
    ] {
        assert!(
            selected(r, path, &[]).iter().any(|t| t == SNAPSHOT_TEST),
            "{path}"
        );
    }
    for path in [
        "apps/admin/src/app/page.tsx",
        "apps/admin/src/lib/client.ts",
    ] {
        assert!(
            selected(r, path, &["--on-uncertain", "ignore"])
                .iter()
                .any(|t| t == SNAPSHOT_TEST),
            "{path} with uncertainty ignored"
        );
    }
    write(
        r,
        "apps/admin/src/app/team/page.tsx",
        "export default function Team() { return null; }\n",
    );
    assert!(
        selected(r, "apps/admin/src/app/team/page.tsx", &[])
            .iter()
            .any(|t| t == SNAPSHOT_TEST)
    );
}

/// The goal: the scan is a dependency on exactly the scanned tree, so the test
/// is selected for a change inside `apps/admin/src` and not for a change in
/// another app's `src`. Needs the value-flow engine to follow the walk (a
/// tracked container returned from a function of the file, and an accumulator
/// parameter with a literal default), and the argument the test passes or its
/// working directory for the root.
#[test]
#[ignore = "the read of the walked files is bounded by names ('src' anywhere), not by the listed \
            tree"]
fn the_scan_is_a_dependency_on_the_scanned_tree() {
    let d = repo();
    let r = d.path();
    let ignore = ["--on-uncertain", "ignore"];
    assert_eq!(
        selected(r, "apps/admin/src/app/page.tsx", &ignore),
        [SNAPSHOT_TEST]
    );
    assert_eq!(
        selected(r, "apps/other/src/o.js", &ignore),
        ["apps/other/src/o.test.js"]
    );
    assert_eq!(
        selected(r, "apps/other/src/o.js", &[]),
        ["apps/other/src/o.test.js"],
        "not on the floor"
    );
}
