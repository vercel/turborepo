import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import * as os from "node:os";
import * as fs from "node:fs";
import { Workspace } from "../js/dist/index.js";

const PNPM_MONOREPO_PATH = path.resolve(__dirname, "./fixtures/monorepo");
const NPM_MONOREPO_PATH = path.resolve(__dirname, "./fixtures/npm-monorepo");

// The set of typed error categories `lockfilePackages` may report. Keep in
// sync with the `LockfileErrorKind` enum in the native module.
const LOCKFILE_ERROR_KINDS = [
  "NoLockfile",
  "LockfileUnreadable",
  "ResolutionFailed",
  "UnparseableEntry",
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
    assert.ok(microdiff, `Expected microdiff, got: ${JSON.stringify(packages)}`);
    assert.equal(microdiff.version, "1.4.0");
  });

  it("returns flat, fully-qualified names without peer closures", async () => {
    const workspace = await Workspace.find(PNPM_MONOREPO_PATH);
    const { packages } = await workspace.lockfilePackages();

    for (const pkg of packages) {
      assert.ok(pkg.name.length > 0, `Expected a name, got: ${JSON.stringify(pkg)}`);
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

  it("reports a typed error instead of throwing when there is no lockfile", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "no-lockfile-"));
    fs.writeFileSync(
      path.join(dir, "package.json"),
      JSON.stringify({
        name: "solo",
        version: "1.0.0",
        packageManager: "pnpm@9.15.9",
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
