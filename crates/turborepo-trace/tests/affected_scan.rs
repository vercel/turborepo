//! A script that scans a source tree, and a test that compares its result with
//! a committed snapshot.
//!
//! The setup: an app's script walks `<app>/src` with a recursive
//! `readdirSync` (the files are collected in an accumulator parameter), reads
//! every code file it finds and derives an inventory from them; a jest test
//! builds the inventory and compares it with a committed JSON file. Adding a
//! page without regenerating the file fails the test. No import leads from the
//! test to the pages: the dependency is the scanned tree.
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

const SCRIPT: &str = r#"import fs from 'node:fs';
import { extname, join, relative } from 'node:path';

const CODE = ['.js', '.jsx', '.ts', '.tsx'];

// Every code file below `dir`, depth first; `found` collects them.
function collect(dir: string, found: string[] = []): string[] {
  const children = fs.readdirSync(dir, { withFileTypes: true });
  for (const child of children) {
    const full = join(dir, child.name);
    if (child.isDirectory()) {
      if (child.name !== 'node_modules' && child.name !== 'fixtures') {
        collect(full, found);
      }
    } else if (CODE.includes(extname(child.name)) && !child.name.endsWith('.d.ts')) {
      found.push(full);
    }
  }
  return found;
}

function contents(files: string[], base: string): Map<string, string> {
  const out = new Map<string, string>();
  files.forEach((file) => out.set(relative(base, file), fs.readFileSync(file, 'utf8')));
  return out;
}

function settings(root: string): string {
  const file = CODE.map((ext) => join(root, `app.config${ext}`)).find(fs.existsSync);
  return file ? fs.readFileSync(file, 'utf8') : '';
}

export function inventory(root: string = process.cwd()): string[] {
  const base = join(root, 'src');
  const files = contents(collect(base), base);
  return [...files.keys(), settings(root)].sort();
}

export function save(root: string = process.cwd()): string {
  const target = join(root, 'inventory.json');
  fs.writeFileSync(target, JSON.stringify(inventory(root), null, 2) + '\n');
  return target;
}
"#;

const TEST: &str = r#"import fs from 'node:fs';
import { resolve } from 'node:path';
import { inventory } from './inventory';

const root = resolve(__dirname, '../..');

test('the committed inventory is current', () => {
  const saved = JSON.parse(fs.readFileSync(resolve(root, 'inventory.json'), 'utf8'));
  expect(inventory(root)).toEqual(saved);
});
"#;

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n");
    write(
        r,
        "apps/console/package.json",
        r#"{"name":"console","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/console/jest.config.js", "module.exports = {};\n");
    write(r, "apps/console/app.config.ts", "export default {};\n");
    write(
        r,
        "apps/console/inventory.json",
        "[\"\", \"app/page.tsx\", \"lib/client.ts\"]\n",
    );
    write(
        r,
        "apps/console/src/app/page.tsx",
        "export default function Page() { return null; }\n",
    );
    write(
        r,
        "apps/console/src/lib/client.ts",
        "export const get = () => 1;\n",
    );
    write(r, "apps/console/scripts/inventory/inventory.ts", SCRIPT);
    write(r, "apps/console/scripts/inventory/inventory.test.ts", TEST);
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

const SNAPSHOT_TEST: &str = "apps/console/scripts/inventory/inventory.test.ts";

/// The snapshot test is selected when a scanned file changes or is added, when
/// the config the script reads changes, and when the snapshot changes: under
/// the default policy, and for the scanned files also when uncertainty is
/// ignored (a real path).
#[test]
fn the_snapshot_test_is_selected_for_changes_in_the_scanned_tree() {
    let d = repo();
    let r = d.path();
    for path in [
        "apps/console/src/app/page.tsx",
        "apps/console/src/lib/client.ts",
        "apps/console/app.config.ts",
        "apps/console/inventory.json",
    ] {
        assert!(
            selected(r, path, &[]).iter().any(|t| t == SNAPSHOT_TEST),
            "{path}"
        );
    }
    for path in [
        "apps/console/src/app/page.tsx",
        "apps/console/src/lib/client.ts",
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
        "apps/console/src/app/team/page.tsx",
        "export default function Team() { return null; }\n",
    );
    assert!(
        selected(r, "apps/console/src/app/team/page.tsx", &[])
            .iter()
            .any(|t| t == SNAPSHOT_TEST)
    );
}

/// The goal: the scan is a dependency on exactly the scanned tree, so the test
/// is selected for a change inside `apps/console/src` and not for a change in
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
        selected(r, "apps/console/src/app/page.tsx", &ignore),
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
