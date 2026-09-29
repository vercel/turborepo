import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import * as os from "node:os";
import * as fs from "node:fs";
import { Workspace } from "../js/dist/index.js";

const PNPM_MONOREPO_PATH = path.resolve(__dirname, "./fixtures/monorepo");
const NPM_MONOREPO_PATH = path.resolve(__dirname, "./fixtures/npm-monorepo");
const NPM_SINGLE_PACKAGE_PATH = path.resolve(
  __dirname,
  "./fixtures/npm-single-package"
);
const LOCKFILE_FIXTURES_PATH = path.resolve(
  __dirname,
  "../../../lockfile-tests/fixtures"
);

// The set of typed error categories `lockfilePackages` may report. Keep in
// sync with the `LockfileErrorKind` enum in the native module.
const LOCKFILE_ERROR_KINDS = [
  "NoLockfile",
  "LockfileUnreadable",
  "ResolutionFailed",
  "UnparseableEntry",
  "UnsupportedNpmLockfileVersion",
  "UnsupportedBunLockfile"
];

describe("packagesFromLockfile", () => {
  it("returns external packages from a pnpm lockfile", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const packages = await workspace.packagesFromLockfile();

    assert.ok(Array.isArray(packages), "Expected an array");
    assert.ok(packages.length > 0, "Expected at least one package");
    assert.ok(
      packages.includes("npm/microdiff@1.4.0"),
      `Expected npm/microdiff@1.4.0, got: ${JSON.stringify(packages)}`
    );
  });

  it("returns an empty array when there are no external dependencies", async () => {
    const workspace = await Workspace.find(NPM_MONOREPO_PATH);
    const packages = await workspace.packagesFromLockfile();

    assert.ok(Array.isArray(packages), "Expected an array");
    assert.equal(packages.length, 0, "Expected no packages");
  });

  it("returns sorted results with npm/ prefix", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const packages = await workspace.packagesFromLockfile();

    for (const pkg of packages) {
      assert.ok(pkg.startsWith("npm/"), `Expected npm/ prefix, got: ${pkg}`);
      assert.match(
        pkg,
        /^npm\/.+@.+$/,
        `Expected format npm/<name>@<version>, got: ${pkg}`
      );
    }

    const sorted = [...packages].sort();
    assert.deepEqual(packages, sorted, "Expected sorted output");
  });

  it("contains no duplicates", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const packages = await workspace.packagesFromLockfile();

    const unique = new Set(packages);
    assert.equal(packages.length, unique.size, "Expected no duplicate entries");
  });
});

describe("lockfilePackages", () => {
  it("returns name/version structs from a pnpm lockfile", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const { packages, errors } = await workspace.lockfilePackages();

    assert.ok(Array.isArray(packages), "Expected a packages array");
    assert.ok(packages.length > 0, "Expected at least one package");
    assert.deepEqual(errors, [], "Expected no parse errors");

    const microdiff = packages.find((pkg) => pkg.name === "microdiff");
    assert.ok(
      microdiff,
      `Expected microdiff, got: ${JSON.stringify(packages)}`
    );
    assert.equal(microdiff.version, "1.4.0");
    assert.equal(microdiff.source, "registry");
  });

  it("returns lockfile and package-manager metadata", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const result = await workspace.lockfilePackages();

    assert.equal(
      result.lockfilePath,
      path.join(PNPM_MONOREPO_PATH, "pnpm-lock.yaml")
    );
    assert.equal(result.lockfileFormat, "pnpm");
    assert.equal(result.lockfileVersion, "9.0");
    assert.equal(result.packageManager, "pnpm9");
    assert.equal(result.packageManagerVersion, "9.15.9");
  });

  it("uses lockfile-test fixtures, including Yarn 1 resolved versions", async () => {
    const workspace = await Workspace.find(
      path.join(LOCKFILE_FIXTURES_PATH, "yarn1-file-dep")
    );
    const result = await workspace.lockfilePackages();

    assert.deepEqual(result.errors, []);
    assert.equal(result.lockfileFormat, "yarn");
    assert.equal(result.lockfileVersion, "1");
    assert.equal(result.packageManager, "yarn");
    assert.equal(result.packageManagerVersion, "1.22.22");
    assert.ok(
      result.packages.some(
        (pkg) =>
          pkg.name === "is-number" &&
          pkg.version === "7.0.0" &&
          pkg.source === "registry"
      )
    );
  });

  it("returns direct and transitive packages from a non-monorepo lockfile", async () => {
    const workspace = await Workspace.find(NPM_SINGLE_PACKAGE_PATH);
    const { packages, errors } = await workspace.lockfilePackages();

    assert.equal(workspace.isMultiPackage, false);
    assert.deepEqual(errors, []);
    assert.deepEqual(packages, [
      { name: "is-number", version: "6.0.0", source: "registry" },
      { name: "is-odd", version: "3.0.1", source: "registry" }
    ]);
  });

  it("returns flat, fully-qualified names without peer closures", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const { packages } = await workspace.lockfilePackages();

    for (const pkg of packages) {
      assert.ok(
        pkg.name.length > 0,
        `Expected a name, got: ${JSON.stringify(pkg)}`
      );
      assert.ok(
        pkg.version.length > 0,
        `Expected a version, got: ${JSON.stringify(pkg)}`
      );
      // No pnpm peer-dependency closures should leak into the version.
      assert.ok(
        !pkg.version.includes("(") && !pkg.version.includes(")"),
        `Expected no closure in version, got: ${pkg.version}`
      );
    }
  });

  it("is sorted and deduplicated", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const { packages } = await workspace.lockfilePackages();

    const keys = packages.map((pkg) => `${pkg.name}@${pkg.version}`);
    assert.deepEqual([...keys].sort(), keys, "Expected sorted output");
    assert.equal(new Set(keys).size, keys.length, "Expected no duplicates");
  });

  it("returns an empty list (not an error) when there are no external deps", async () => {
    const workspace = await Workspace.find(NPM_MONOREPO_PATH);
    const { packages, errors } = await workspace.lockfilePackages();

    assert.deepEqual(packages, [], "Expected no packages");
    assert.deepEqual(errors, [], "Expected no parse errors");
  });

  it("reports typed errors for unsupported npm v1 and bun.lockb", async () => {
    const npmDir = fs.mkdtempSync(path.join(os.tmpdir(), "npm-v1-lockfile-"));
    fs.writeFileSync(
      path.join(npmDir, "package.json"),
      JSON.stringify({
        name: "npm-v1",
        version: "1.0.0",
        packageManager: "npm@6.14.18",
        dependencies: { lodash: "^4.17.21" }
      })
    );
    fs.writeFileSync(
      path.join(npmDir, "package-lock.json"),
      JSON.stringify({
        name: "npm-v1",
        version: "1.0.0",
        lockfileVersion: 1,
        dependencies: { lodash: { version: "4.17.21" } }
      })
    );
    const npmResult = await (await Workspace.find(npmDir)).lockfilePackages();
    assert.equal(npmResult.errors[0]?.kind, "UnsupportedNpmLockfileVersion");

    const bunDir = fs.mkdtempSync(path.join(os.tmpdir(), "bun-lockb-"));
    fs.writeFileSync(
      path.join(bunDir, "package.json"),
      JSON.stringify({
        name: "bun-lockb",
        version: "1.0.0",
        packageManager: "bun@1.2.0"
      })
    );
    fs.writeFileSync(path.join(bunDir, "bun.lockb"), "binary");
    const bunResult = await (await Workspace.find(bunDir)).lockfilePackages();
    assert.equal(bunResult.errors[0]?.kind, "UnsupportedBunLockfile");
    assert.equal(bunResult.lockfilePath, path.join(bunDir, "bun.lockb"));
  });

  it("reports a typed error instead of throwing when there is no lockfile", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "no-lockfile-"));
    fs.writeFileSync(
      path.join(dir, "package.json"),
      JSON.stringify({
        name: "solo",
        version: "1.0.0",
        packageManager: "pnpm@9.15.9"
      })
    );

    const workspace = await Workspace.find(dir);
    const { packages, errors } = await workspace.lockfilePackages();

    assert.deepEqual(packages, [], "Expected no packages");
    assert.equal(errors.length, 1, "Expected exactly one error");
    assert.equal(errors[0].kind, "NoLockfile");
    assert.ok(errors[0].message.length > 0, "Expected an error message");

    // Every reported error carries a known, typed category so callers can
    // group failures in metrics.
    for (const error of errors) {
      assert.ok(
        LOCKFILE_ERROR_KINDS.includes(error.kind),
        `Unexpected error kind: ${error.kind}`
      );
    }
  });
});

describe("lockfilePackages with skipPackageGraph", () => {
  it("matches the package-graph result for a pnpm monorepo", async () => {
    const withGraph = await (
      await Workspace.find(PNPM_MONOREPO_PATH)
    ).lockfilePackages();
    const withoutGraph = await (
      await Workspace.find(PNPM_MONOREPO_PATH, { skipPackageGraph: true })
    ).lockfilePackages();

    assert.ok(
      withoutGraph.packages.length > 0,
      "Expected at least one package"
    );
    assert.deepEqual(withoutGraph, withGraph);
  });

  it("matches the package-graph result for a Yarn 1 fixture", async () => {
    const dir = path.join(LOCKFILE_FIXTURES_PATH, "yarn1-file-dep");
    const withGraph = await (await Workspace.find(dir)).lockfilePackages();
    const withoutGraph = await (
      await Workspace.find(dir, { skipPackageGraph: true })
    ).lockfilePackages();

    assert.deepEqual(withoutGraph, withGraph);
  });

  it("still works for single-package repos", async () => {
    const withGraph = await (
      await Workspace.find(NPM_SINGLE_PACKAGE_PATH)
    ).lockfilePackages();
    const withoutGraph = await (
      await Workspace.find(NPM_SINGLE_PACKAGE_PATH, { skipPackageGraph: true })
    ).lockfilePackages();

    assert.deepEqual(withoutGraph, withGraph);
  });

  it("rejects graph-backed methods with a clear error", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH, {
      skipPackageGraph: true
    });
    assert.equal(workspace.isMultiPackage, true);
    assert.equal(workspace.packageManager.version, "9.15.9");

    for (const call of [
      () => workspace.findPackages(),
      () => workspace.findPackagesWithGraph(),
      () => workspace.packagesFromLockfile(),
      () => workspace.affectedPackages([]),
      () => workspace.findPackageByPath("apps/app/index.js")
    ]) {
      await assert.rejects(call, /skipPackageGraph/);
    }
  });
});

describe("packageManager version", () => {
  it("exposes the version from the packageManager field", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    assert.ok(
      workspace.packageManager.name.startsWith("pnpm"),
      `Expected a pnpm variant, got: ${workspace.packageManager.name}`
    );
    assert.equal(workspace.packageManager.version, "9.15.9");
  });

  it("falls back to devEngines.packageManager.version", async () => {
    const workspace = await Workspace.find(NPM_MONOREPO_PATH);
    assert.equal(workspace.packageManager.name, "npm");
    assert.equal(workspace.packageManager.version, "10.5.0");
  });
});
