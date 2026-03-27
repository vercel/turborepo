import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace } from "../js/dist/index.js";

const PNPM_MONOREPO_PATH = path.resolve(__dirname, "./fixtures/monorepo");
const NPM_MONOREPO_PATH = path.resolve(__dirname, "./fixtures/npm-monorepo");

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
