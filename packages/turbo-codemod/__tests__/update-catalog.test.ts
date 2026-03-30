import path from "node:path";
import fs from "fs-extra";
import { describe, it, expect } from "@jest/globals";
import { setupTestFixtures } from "@turbo/test-utils";
import {
  detectCatalog,
  updateCatalogVersion
} from "../src/commands/migrate/steps/update-catalog";

describe("detectCatalog", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "get-turbo-upgrade-command"
  });

  it("returns undefined for normal (non-catalog) pnpm devDependency", () => {
    const { root } = useFixture({ fixture: "pnpm-workspaces-dev-install" });
    const result = detectCatalog({ root, packageManager: "pnpm" });
    expect(result).toBeUndefined();
  });

  it("returns undefined for normal (non-catalog) npm dependency", () => {
    const { root } = useFixture({ fixture: "normal-workspaces" });
    const result = detectCatalog({ root, packageManager: "npm" });
    expect(result).toBeUndefined();
  });

  it("returns undefined when turbo is not in package.json", () => {
    const { root } = useFixture({ fixture: "no-turbo" });
    const result = detectCatalog({ root, packageManager: "pnpm" });
    expect(result).toBeUndefined();
  });

  it("returns undefined when package.json does not exist", () => {
    const { root } = useFixture({ fixture: "no-package" });
    const result = detectCatalog({ root, packageManager: "pnpm" });
    expect(result).toBeUndefined();
  });

  it("returns undefined for npm (catalogs not supported)", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const result = detectCatalog({ root, packageManager: "npm" });
    expect(result).toBeUndefined();
  });

  it("detects pnpm default catalog reference", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const result = detectCatalog({ root, packageManager: "pnpm" });
    expect(result).toEqual({
      catalogName: null,
      catalogFile: path.join(root, "pnpm-workspace.yaml"),
      installType: "devDependencies"
    });
  });

  it("detects pnpm named catalog reference", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-named" });
    const result = detectCatalog({ root, packageManager: "pnpm" });
    expect(result).toEqual({
      catalogName: "build",
      catalogFile: path.join(root, "pnpm-workspace.yaml"),
      installType: "devDependencies"
    });
  });

  it("detects yarn default catalog reference", () => {
    const { root } = useFixture({ fixture: "yarn-catalog-default" });
    const result = detectCatalog({ root, packageManager: "yarn" });
    expect(result).toEqual({
      catalogName: null,
      catalogFile: path.join(root, ".yarnrc.yml"),
      installType: "devDependencies"
    });
  });
});

describe("updateCatalogVersion", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "get-turbo-upgrade-command"
  });

  it("updates the default catalog in pnpm-workspace.yaml", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const catalogFile = path.join(root, "pnpm-workspace.yaml");
    const catalogInfo = {
      catalogName: null,
      catalogFile,
      installType: "devDependencies" as const
    };

    const updated = updateCatalogVersion({
      catalogInfo,
      version: "2.9.0"
    });

    expect(updated).toBe(true);
    const content = fs.readFileSync(catalogFile, "utf8");
    expect(content).toContain('turbo: "^2.9.0"');
    expect(content).not.toContain("^2.0.0");
  });

  it("updates a named catalog in pnpm-workspace.yaml", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-named" });
    const catalogFile = path.join(root, "pnpm-workspace.yaml");
    const catalogInfo = {
      catalogName: "build",
      catalogFile,
      installType: "devDependencies" as const
    };

    const updated = updateCatalogVersion({
      catalogInfo,
      version: "2.9.0"
    });

    expect(updated).toBe(true);
    const content = fs.readFileSync(catalogFile, "utf8");
    expect(content).toContain('turbo: "^2.9.0"');
  });

  it("updates the default catalog in .yarnrc.yml", () => {
    const { root } = useFixture({ fixture: "yarn-catalog-default" });
    const catalogFile = path.join(root, ".yarnrc.yml");
    const catalogInfo = {
      catalogName: null,
      catalogFile,
      installType: "devDependencies" as const
    };

    const updated = updateCatalogVersion({
      catalogInfo,
      version: "2.9.0"
    });

    expect(updated).toBe(true);
    const content = fs.readFileSync(catalogFile, "utf8");
    expect(content).toContain('turbo: "^2.9.0"');
  });

  it("preserves tilde (~) version prefix", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const catalogFile = path.join(root, "pnpm-workspace.yaml");

    // Rewrite fixture with tilde prefix
    const content = fs.readFileSync(catalogFile, "utf8");
    fs.writeFileSync(catalogFile, content.replace("^2.0.0", "~2.0.0"));

    const catalogInfo = {
      catalogName: null,
      catalogFile,
      installType: "devDependencies" as const
    };

    updateCatalogVersion({ catalogInfo, version: "2.9.0" });

    const updated = fs.readFileSync(catalogFile, "utf8");
    expect(updated).toContain('turbo: "~2.9.0"');
  });

  it("preserves exact version (no prefix)", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const catalogFile = path.join(root, "pnpm-workspace.yaml");

    // Rewrite fixture with no prefix
    const content = fs.readFileSync(catalogFile, "utf8");
    fs.writeFileSync(catalogFile, content.replace("^2.0.0", "2.0.0"));

    const catalogInfo = {
      catalogName: null,
      catalogFile,
      installType: "devDependencies" as const
    };

    updateCatalogVersion({ catalogInfo, version: "2.9.0" });

    const updated = fs.readFileSync(catalogFile, "utf8");
    expect(updated).toContain('turbo: "2.9.0"');
    expect(updated).not.toMatch(/turbo: "\^2\.9\.0"/);
  });

  it("returns false when version is already up to date", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const catalogFile = path.join(root, "pnpm-workspace.yaml");
    const catalogInfo = {
      catalogName: null,
      catalogFile,
      installType: "devDependencies" as const
    };

    const updated = updateCatalogVersion({
      catalogInfo,
      version: "2.0.0"
    });

    expect(updated).toBe(false);
  });

  it("preserves other YAML content when updating", () => {
    const { root } = useFixture({ fixture: "pnpm-catalog-default" });
    const catalogFile = path.join(root, "pnpm-workspace.yaml");
    const catalogInfo = {
      catalogName: null,
      catalogFile,
      installType: "devDependencies" as const
    };

    updateCatalogVersion({ catalogInfo, version: "2.9.0" });

    const content = fs.readFileSync(catalogFile, "utf8");
    // packages section should still be intact
    expect(content).toContain('- "apps/*"');
    expect(content).toContain('- "packages/*"');
  });
});
