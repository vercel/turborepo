/**
 * check-lockfiles.ts
 *
 * End-to-end validation that turborepo's lockfile pruning produces lockfiles
 * that pass frozen-lockfile installs for every supported package manager.
 *
 * Each fixture under lockfile-tests/fixtures/ is a complete monorepo with a
 * meta.json describing the package manager, frozen install command, and which
 * workspaces to prune to. This script copies each fixture into a temp
 * directory, runs `turbo prune` for every target workspace, then runs the
 * frozen install to prove the pruned lockfile is valid.
 *
 * Usage:
 *   pnpm check-lockfiles                                         # All fixtures
 *   pnpm check-lockfiles --fixture pnpm8                         # Match fixture name
 *   pnpm check-lockfiles --pm pnpm                               # Only pnpm fixtures
 *   pnpm check-lockfiles --fixture pnpm8 --workspace a           # Specific target
 *   pnpm check-lockfiles --turbo-path ./path/to/turbo            # Custom turbo binary
 */

import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import type { PackageManagerType, TestCase } from "./types";
import { LocalRunner } from "./runners/local";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// ---------------------------------------------------------------------------
// Meta.json schema (committed alongside each fixture)
// ---------------------------------------------------------------------------

interface FixtureMeta {
  packageManager: PackageManagerType;
  packageManagerVersion: string;
  lockfileName: string;
  frozenInstallCommand: string[];
  /** Workspace names where pruning or frozen install is known to fail. */
  expectedFailures?: string[];
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

interface CliArgs {
  fixture?: string;
  pm?: PackageManagerType;
  workspace?: string;
  turboPath?: string;
  concurrency: number;
}

function parseArgs(): CliArgs {
  const args: CliArgs = { concurrency: 10 };
  const argv = process.argv.slice(2);

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = argv[i + 1];

    if (arg === "--fixture" && next) {
      args.fixture = next;
      i++;
    } else if (arg === "--pm" && next) {
      const valid: PackageManagerType[] = ["npm", "pnpm", "yarn-berry", "bun"];
      if (!valid.includes(next as PackageManagerType)) {
        console.error(
          `Invalid --pm: ${next}. Must be one of: ${valid.join(", ")}`
        );
        process.exit(1);
      }
      args.pm = next as PackageManagerType;
      i++;
    } else if (arg === "--workspace" && next) {
      args.workspace = next;
      i++;
    } else if (arg === "--turbo-path" && next) {
      args.turboPath = next;
      i++;
    } else if (arg === "--concurrency" && next) {
      args.concurrency = parseInt(next, 10);
      i++;
    }
  }

  return args;
}

// ---------------------------------------------------------------------------
// Fixture discovery
// ---------------------------------------------------------------------------

const FIXTURES_DIR = path.resolve(__dirname, "fixtures");

interface DiscoveredFixture {
  name: string;
  dir: string;
  meta: FixtureMeta;
  pruneTargets: string[];
}

/**
 * Discovers workspace package names by finding all package.json files in the
 * fixture directory (excluding root and node_modules).
 */
function discoverPruneTargets(fixtureDir: string): string[] {
  const rootPkgPath = path.join(fixtureDir, "package.json");
  const rootName = fs.existsSync(rootPkgPath)
    ? JSON.parse(fs.readFileSync(rootPkgPath, "utf-8")).name
    : null;

  const targets: string[] = [];

  function walk(dir: string) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (
        entry.name === "node_modules" ||
        entry.name === ".git" ||
        entry.name === "out"
      )
        continue;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        const pkgPath = path.join(full, "package.json");
        if (fs.existsSync(pkgPath)) {
          const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf-8"));
          if (pkg.name && pkg.name !== rootName) {
            targets.push(pkg.name);
          }
        }
        walk(full);
      }
    }
  }

  walk(fixtureDir);
  return targets.sort();
}

function discoverFixtures(args: CliArgs): DiscoveredFixture[] {
  if (!fs.existsSync(FIXTURES_DIR)) {
    console.error(
      `Fixtures directory not found: ${FIXTURES_DIR}\nRun \`npx tsx generate-fixtures.ts\` first.`
    );
    process.exit(1);
  }

  const entries = fs.readdirSync(FIXTURES_DIR, { withFileTypes: true });
  const fixtures: DiscoveredFixture[] = [];

  for (const entry of entries) {
    if (!entry.isDirectory()) continue;

    const metaPath = path.join(FIXTURES_DIR, entry.name, "meta.json");
    if (!fs.existsSync(metaPath)) continue;

    const meta: FixtureMeta = JSON.parse(fs.readFileSync(metaPath, "utf-8"));

    if (args.pm && meta.packageManager !== args.pm) continue;
    if (args.fixture && !entry.name.includes(args.fixture)) continue;

    const fixtureDir = path.join(FIXTURES_DIR, entry.name);
    fixtures.push({
      name: entry.name,
      dir: fixtureDir,
      meta,
      pruneTargets: discoverPruneTargets(fixtureDir)
    });
  }

  return fixtures.sort((a, b) => a.name.localeCompare(b.name));
}

// ---------------------------------------------------------------------------
// Test case generation
// ---------------------------------------------------------------------------

function buildTestCases(
  fixtures: DiscoveredFixture[],
  args: CliArgs
): { cases: TestCase[]; expectedFailures: Set<string> } {
  const cases: TestCase[] = [];
  const expectedFailures = new Set<string>();

  for (const fixture of fixtures) {
    const targets = args.workspace
      ? fixture.pruneTargets.filter((t) => t === args.workspace)
      : fixture.pruneTargets;

    const expectedSet = new Set(fixture.meta.expectedFailures ?? []);

    for (const target of targets) {
      const label = `${fixture.name} → ${target}`;
      const isExpected = expectedSet.has(target);
      if (isExpected) {
        expectedFailures.add(label);
      }
      cases.push({
        fixture: {
          filename: fixture.name,
          filepath: fixture.dir,
          packageManager: fixture.meta.packageManager,
          lockfileName: fixture.meta.lockfileName,
          frozenInstallCommand: fixture.meta.frozenInstallCommand,
          packageManagerVersion: fixture.meta.packageManagerVersion
        },
        targetWorkspace: { name: target },
        label,
        expectedFailure: isExpected
      });
    }
  }

  return { cases, expectedFailures };
}

// ---------------------------------------------------------------------------
// Turbo binary resolution
// ---------------------------------------------------------------------------

function findTurboBinary(args: CliArgs): string {
  if (args.turboPath) {
    const resolved = path.resolve(args.turboPath);
    if (!fs.existsSync(resolved)) {
      console.error(`Turbo binary not found at: ${resolved}`);
      process.exit(1);
    }
    return resolved;
  }

  const candidates = [
    path.resolve(__dirname, "../target/debug/turbo"),
    path.resolve(__dirname, "../target/release/turbo"),
    path.resolve(__dirname, "../target/release-turborepo/turbo")
  ];

  for (const c of candidates) {
    if (fs.existsSync(c)) {
      console.log(`Found turbo binary: ${c}`);
      return c;
    }
  }

  console.error(
    "No turbo binary found. Build with `cargo build` or pass --turbo-path."
  );
  console.error("Searched:", candidates.join(", "));
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Concurrency helper
// ---------------------------------------------------------------------------

async function runWithConcurrency<T, R>(
  items: T[],
  concurrency: number,
  fn: (item: T) => Promise<R>
): Promise<R[]> {
  const results = Array.from<R>({ length: items.length });
  let nextIndex = 0;

  async function worker() {
    while (nextIndex < items.length) {
      const index = nextIndex++;
      results[index] = await fn(items[index]);
    }
  }

  const workers = Array.from(
    { length: Math.min(concurrency, items.length) },
    () => worker()
  );
  await Promise.all(workers);
  return results;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  const totalStart = Date.now();
  const args = parseArgs();

  const fixtures = discoverFixtures(args);
  console.log(`Found ${fixtures.length} fixtures\n`);

  const { cases: testCases, expectedFailures } = buildTestCases(fixtures, args);
  console.log(`${testCases.length} test cases\n`);

  if (expectedFailures.size > 0) {
    console.log(`Expected failures (known bugs): ${expectedFailures.size}`);
    for (const label of expectedFailures) {
      console.log(`  ${label}`);
    }
    console.log();
  }

  if (testCases.length === 0) {
    console.log("Nothing to run. Check your filters.");
    return;
  }

  const turboBinary = findTurboBinary(args);
  const runner = new LocalRunner();

  console.log("\nTest plan:");
  for (const tc of testCases) {
    console.log(`  ${tc.label}`);
  }

  console.log(
    `\nRunning ${testCases.length} tests (concurrency: ${args.concurrency})...\n`
  );

  const results = await runWithConcurrency(testCases, args.concurrency, (tc) =>
    runner.runTestCase(tc, turboBinary)
  );

  // Summary
  const totalDuration = Date.now() - totalStart;
  console.log("\n" + "=".repeat(60));
  console.log("Results Summary");
  console.log("=".repeat(60) + "\n");

  const passed = results.filter((r) => r.success);
  const allFailures = results.filter((r) => !r.success);
  const expectedFailureResults = allFailures.filter((r) =>
    expectedFailures.has(r.label)
  );
  const unexpectedFailures = allFailures.filter(
    (r) => !expectedFailures.has(r.label)
  );
  const unexpectedPasses = passed.filter((r) => expectedFailures.has(r.label));

  for (const r of results) {
    let status: string;
    if (r.success && expectedFailures.has(r.label)) {
      status = "PASS (was expected to fail!)";
    } else if (r.success) {
      status = "PASS";
    } else if (expectedFailures.has(r.label)) {
      status = r.pruneSuccess
        ? "EXPECTED FAIL (install)"
        : "EXPECTED FAIL (prune)";
    } else {
      status = r.pruneSuccess ? "FAIL (install)" : "FAIL (prune)";
    }
    console.log(
      `  ${status} ${r.label} (${(r.durationMs / 1000).toFixed(1)}s)`
    );
  }

  console.log(`\nTotal: ${results.length} tests`);
  console.log(`  Passed:            ${passed.length}`);
  console.log(`  Expected failures: ${expectedFailureResults.length}`);
  console.log(`  Unexpected fails:  ${unexpectedFailures.length}`);
  console.log(`  Duration:          ${(totalDuration / 1000).toFixed(1)}s`);

  if (unexpectedPasses.length > 0) {
    console.log(
      "\nTests that were expected to fail but now PASS (update meta.json!):"
    );
    for (const r of unexpectedPasses) {
      console.log(`  ${r.label}`);
    }
  }

  if (unexpectedFailures.length > 0) {
    console.log("\nUnexpected failure details:\n");
    for (const r of unexpectedFailures) {
      console.log(`--- ${r.label} ---`);
      if (r.error) {
        const truncated =
          r.error.length > 2000
            ? r.error.slice(0, 2000) + "\n... (truncated)"
            : r.error;
        console.log(truncated);
      }
      console.log();
    }
    process.exit(1);
  }

  if (expectedFailureResults.length > 0) {
    console.log(
      `\n${expectedFailureResults.length} known failures (expected). Fix these and remove from expectedFailures in meta.json.`
    );
  }

  console.log("\nNo unexpected failures!");
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
