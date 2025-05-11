import path from "node:path";
import fs from "node:fs";
import { RuleTester } from "eslint";
import { describe, expect, it, beforeEach, afterEach } from "@jest/globals";
import type { SchemaV1 } from "@turbo/types";
import { RULES } from "../../../../lib/constants";
import rule from "../../../../lib/rules/no-undeclared-env-vars";
import { Project } from "../../../../lib/utils/calculate-inputs";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020, sourceType: "module" },
});

const cwd = path.join(__dirname, "../../../../__fixtures__/workspace-configs");
const webFilename = path.join(cwd, "/apps/web/index.js");

describe("Project reload functionality", () => {
  let project: Project;
  let originalTurboJson: string;

  beforeEach(() => {
    project = new Project(cwd);
    // Store original turbo.json content for restoration
    const turboJsonPath = path.join(cwd, "turbo.json");
    originalTurboJson = fs.readFileSync(turboJsonPath, "utf8");
  });

  afterEach(() => {
    // Restore original turbo.json content
    const turboJsonPath = path.join(cwd, "turbo.json");
    fs.writeFileSync(turboJsonPath, originalTurboJson);
  });

  it("should reload workspace configurations when called", () => {
    const initialConfigs = [...project.allConfigs];

    // Call reload
    project.reload();

    // Verify that configurations were reloaded
    expect(project.allConfigs).not.toBe(initialConfigs);
    expect(project.allConfigs.length).toBe(initialConfigs.length);

    // Verify that project root and workspaces were updated
    expect(project.projectRoot).toBeDefined();
    expect(project.projectWorkspaces.length).toBeGreaterThan(0);
  });

  it("should regenerate key and test configurations after reload", () => {
    const initialKey = project._key;
    const initialTest = project._test;

    // Call reload
    project.reload();

    // Verify that key and test configurations were regenerated
    expect(project._key).not.toBe(initialKey);
    expect(project._test).not.toBe(initialTest);
  });

  it("should detect changes in turbo.json after reload", () => {
    const turboJsonPath = path.join(cwd, "turbo.json");
    const initialConfig = project.projectRoot?.turboConfig;

    // Modify turbo.json
    const modifiedConfig: SchemaV1 = {
      ...(JSON.parse(originalTurboJson) as SchemaV1),
      pipeline: {
        ...(JSON.parse(originalTurboJson) as SchemaV1).pipeline,
        newTask: {
          outputs: [],
        },
      },
    };
    fs.writeFileSync(turboJsonPath, JSON.stringify(modifiedConfig, null, 2));

    // Call reload
    project.reload();

    // Verify that the new configuration is loaded
    expect(project.projectRoot?.turboConfig).not.toEqual(initialConfig);
    expect(project.projectRoot?.turboConfig).toEqual(modifiedConfig);
  });

  it("should handle invalid turbo.json gracefully", () => {
    const turboJsonPath = path.join(cwd, "turbo.json");

    // Write invalid JSON
    fs.writeFileSync(turboJsonPath, "invalid json");

    // Call reload - should not throw
    expect(() => {
      project.reload();
    }).not.toThrow();

    // Verify that the project still has a valid state
    expect(project.projectRoot).toBeDefined();
    expect(project.projectWorkspaces.length).toBeGreaterThan(0);
  });

  it("should maintain consistent state after multiple reloads", () => {
    const initialConfigs = [...project.allConfigs];

    // Perform multiple reloads
    project.reload();
    project.reload();
    project.reload();

    // Verify that the final state is consistent
    expect(project.allConfigs.length).toBe(initialConfigs.length);
    expect(project.projectRoot).toBeDefined();
    expect(project.projectWorkspaces.length).toBeGreaterThan(0);
  });
});

// Test that the reload functionality works with the ESLint rule
ruleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `
        const { ENV_2 } = import.meta.env;
      `,
      options: [{ cwd }],
      filename: webFilename,
    },
  ],
  invalid: [
    {
      code: `
        const { ENV_3 } = import.meta.env;
      `,
      options: [{ cwd }],
      filename: webFilename,
      errors: [
        {
          message:
            "ENV_3 is not listed as a dependency in the root turbo.json or workspace (apps/web) turbo.json",
        },
      ],
    },
  ],
});
