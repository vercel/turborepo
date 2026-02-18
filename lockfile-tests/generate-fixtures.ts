/**
 * generate-fixtures.ts
 *
 * Reads raw lockfiles from a source directory and generates complete, committed
 * fixture directories under lockfile-tests/fixtures/.
 *
 * Each generated fixture is a minimal monorepo with:
 *   - Root package.json (with packageManager, workspaces, deps)
 *   - Workspace package.json files
 *   - The lockfile (renamed to its canonical name)
 *   - pnpm-workspace.yaml / .yarnrc.yml as needed
 *   - turbo.json
 *   - meta.json (test metadata: prune targets, frozen install cmd, etc.)
 *   - Empty patch file stubs if the fixture uses patches
 *
 * Usage:
 *   npx tsx generate-fixtures.ts --from /path/to/raw/lockfiles
 *   npx tsx generate-fixtures.ts --from /path/to/raw/lockfiles --fixture pnpm8
 */

import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import type { FixtureInfo, PackageManagerType } from "./parsers/types";
import { parseNpmLockfile } from "./parsers/npm";
import { parsePnpmLockfile } from "./parsers/pnpm";
import { parseBerryLockfile } from "./parsers/berry";
import { parseBunLockfile } from "./parsers/bun";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const OUTPUT_DIR = path.resolve(__dirname, "fixtures");

const SKIP_FIXTURES = new Set(["yarn1.lock", "yarn1full.lock", "gh_8849.lock"]);

function classifyFixture(filename: string): PackageManagerType | null {
  if (SKIP_FIXTURES.has(filename)) return null;
  if (filename.endsWith(".json")) return "npm";
  if (filename.endsWith(".yaml") || filename.startsWith("pnpm")) return "pnpm";
  if (
    filename.startsWith("berry") ||
    filename.startsWith("minimal-berry") ||
    filename.startsWith("robust-berry") ||
    filename.startsWith("yarn4")
  )
    return "yarn-berry";
  if (filename.includes("bun")) return "bun";
  return null;
}

function fixtureDirectoryName(filename: string): string {
  return filename.replace(/\.(json|yaml|lock)$/, "").replace(/\./g, "-");
}

function deriveWorkspaceGlobs(paths: string[]): string[] {
  const globs = new Set<string>();
  for (const p of paths) {
    const parts = p.split("/");
    if (parts.length === 2) {
      globs.add(`${parts[0]}/*`);
    } else if (parts.length === 1) {
      globs.add(p);
    } else {
      globs.add(`${parts[0]}/${parts[1]}/*`);
    }
  }
  return Array.from(globs);
}

function generateFixtureDir(fixture: FixtureInfo): void {
  const dirName = fixtureDirectoryName(fixture.filename);
  const outDir = path.join(OUTPUT_DIR, dirName);

  // Wipe and recreate
  if (fs.existsSync(outDir)) {
    fs.rmSync(outDir, { recursive: true });
  }
  fs.mkdirSync(outDir, { recursive: true });

  const rootWorkspace = fixture.workspaces.find((w) => w.path === ".");
  const nonRootWorkspaces = fixture.workspaces.filter((w) => w.path !== ".");

  // meta.json — test runner reads this
  const meta = {
    packageManager: fixture.packageManager,
    packageManagerVersion: fixture.packageManagerVersion,
    lockfileName: fixture.lockfileName,
    frozenInstallCommand: fixture.frozenInstallCommand,
    pruneTargets: nonRootWorkspaces.map((w) => w.name),
    sourceFixture: fixture.filename
  };
  writeJson(path.join(outDir, "meta.json"), meta);

  // Root package.json
  const rootPkg: Record<string, unknown> = {
    name: rootWorkspace?.name || "test-monorepo",
    version: "0.0.0",
    private: true,
    packageManager: fixture.packageManagerVersion
  };

  if (fixture.packageManager !== "pnpm") {
    const globs = fixture.rootExtras.workspaces;
    if (globs && Array.isArray(globs) && globs.length > 0) {
      rootPkg.workspaces = globs;
    } else {
      rootPkg.workspaces = deriveWorkspaceGlobs(
        nonRootWorkspaces.map((w) => w.path)
      );
    }
  }

  if (rootWorkspace) {
    if (Object.keys(rootWorkspace.dependencies).length > 0) {
      rootPkg.dependencies = { ...rootWorkspace.dependencies };
    }
    if (Object.keys(rootWorkspace.devDependencies).length > 0) {
      rootPkg.devDependencies = { ...rootWorkspace.devDependencies };
    }
  }

  if (fixture.packageManager === "pnpm") {
    const pnpmConfig: Record<string, unknown> = {};
    if (fixture.rootExtras.pnpm) {
      Object.assign(
        pnpmConfig,
        fixture.rootExtras.pnpm as Record<string, unknown>
      );
    }
    if (fixture.hasPatches && parseFloat(fixture.lockfileVersion) >= 9) {
      const patchedDeps: Record<string, string> = {};
      for (const patchFile of fixture.patchFiles) {
        const basename = patchFile.split("/").pop() || "";
        const pkgId = basename.replace(/\.patch$/, "");
        patchedDeps[pkgId] = patchFile;
      }
      pnpmConfig.patchedDependencies = patchedDeps;
    }
    if (Object.keys(pnpmConfig).length > 0) {
      rootPkg.pnpm = pnpmConfig;
    }
  }

  if (
    fixture.packageManager === "yarn-berry" &&
    fixture.rootExtras.resolutions
  ) {
    rootPkg.resolutions = fixture.rootExtras.resolutions;
  }

  writeJson(path.join(outDir, "package.json"), rootPkg);

  // pnpm-workspace.yaml
  if (fixture.packageManager === "pnpm") {
    const globs = deriveWorkspaceGlobs(nonRootWorkspaces.map((w) => w.path));
    let content = `packages:\n${globs.map((g) => `  - "${g}"`).join("\n")}\n`;

    const catalogs = fixture.rootExtras.catalogs as
      | Record<string, Record<string, string>>
      | undefined;
    if (catalogs) {
      const defaultCatalog = catalogs["default"];
      if (defaultCatalog && Object.keys(defaultCatalog).length > 0) {
        content += "\ncatalog:\n";
        for (const [pkg, spec] of Object.entries(defaultCatalog)) {
          content += `  ${pkg}: "${spec}"\n`;
        }
      }
      const namedCatalogs = Object.entries(catalogs).filter(
        ([name]) => name !== "default"
      );
      if (namedCatalogs.length > 0) {
        content += "\ncatalogs:\n";
        for (const [catalogName, entries] of namedCatalogs) {
          content += `  ${catalogName}:\n`;
          for (const [pkg, spec] of Object.entries(entries)) {
            content += `    ${pkg}: "${spec}"\n`;
          }
        }
      }
    }

    fs.writeFileSync(path.join(outDir, "pnpm-workspace.yaml"), content);
  }

  // .yarnrc.yml
  if (fixture.packageManager === "yarn-berry") {
    fs.writeFileSync(
      path.join(outDir, ".yarnrc.yml"),
      "nodeLinker: node-modules\n"
    );
  }

  // turbo.json
  writeJson(path.join(outDir, "turbo.json"), { tasks: { build: {} } });

  // Workspace package.jsons
  for (const ws of nonRootWorkspaces) {
    const pkg: Record<string, unknown> = {
      name: ws.name,
      version: "0.0.0",
      private: true
    };
    if (Object.keys(ws.dependencies).length > 0) {
      pkg.dependencies = { ...ws.dependencies };
    }
    if (Object.keys(ws.devDependencies).length > 0) {
      pkg.devDependencies = { ...ws.devDependencies };
    }
    if (Object.keys(ws.peerDependencies).length > 0) {
      pkg.peerDependencies = ws.peerDependencies;
    }

    const wsDir = path.join(outDir, ws.path);
    fs.mkdirSync(wsDir, { recursive: true });
    writeJson(path.join(wsDir, "package.json"), pkg);
  }

  // Patch file stubs
  for (const patchFile of fixture.patchFiles) {
    const patchPath = path.join(outDir, patchFile);
    fs.mkdirSync(path.dirname(patchPath), { recursive: true });
    fs.writeFileSync(patchPath, "");
  }

  // The lockfile (copy and rename to canonical name)
  const lockfileContent = fs.readFileSync(fixture.filepath, "utf-8");
  fs.writeFileSync(path.join(outDir, fixture.lockfileName), lockfileContent);

  console.log(
    `  ${dirName}/ (${fixture.packageManager}, ${nonRootWorkspaces.length} workspaces)`
  );
}

function writeJson(filepath: string, data: unknown): void {
  fs.writeFileSync(filepath, JSON.stringify(data, null, 2) + "\n");
}

function main(): void {
  const filterArg = process.argv.find((_, i, a) => a[i - 1] === "--fixture");
  const fromArg = process.argv.find((_, i, a) => a[i - 1] === "--from");

  if (!fromArg) {
    console.error(
      "Usage: npx tsx generate-fixtures.ts --from /path/to/raw/lockfiles"
    );
    process.exit(1);
  }

  const rawDir = path.resolve(fromArg);
  if (!fs.existsSync(rawDir)) {
    console.error(`Source directory not found: ${rawDir}`);
    process.exit(1);
  }

  console.log(`Generating lockfile test fixtures from ${rawDir}...\n`);
  fs.mkdirSync(OUTPUT_DIR, { recursive: true });

  const entries = fs.readdirSync(rawDir);
  let count = 0;

  for (const entry of entries) {
    if (filterArg && !entry.includes(filterArg)) continue;

    const pm = classifyFixture(entry);
    if (!pm) continue;

    const filepath = path.join(rawDir, entry);
    if (!fs.statSync(filepath).isFile()) continue;

    const content = fs.readFileSync(filepath, "utf-8");

    try {
      let fixture: FixtureInfo;
      switch (pm) {
        case "npm":
          fixture = parseNpmLockfile(content, entry, filepath);
          break;
        case "pnpm":
          fixture = parsePnpmLockfile(content, entry, filepath);
          break;
        case "yarn-berry":
          fixture = parseBerryLockfile(content, entry, filepath);
          break;
        case "bun":
          fixture = parseBunLockfile(content, entry, filepath);
          break;
      }

      generateFixtureDir(fixture);
      count++;
    } catch (err) {
      console.warn(`  SKIP ${entry}: ${err}`);
    }
  }

  console.log(`\nGenerated ${count} fixture directories in ${OUTPUT_DIR}`);
}

main();
