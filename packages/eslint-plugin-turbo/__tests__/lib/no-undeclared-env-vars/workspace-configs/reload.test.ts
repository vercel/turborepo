import path from "node:path";
import { RuleTester } from "eslint";
import { describe, expect, it } from "@jest/globals";
import { RULES } from "../../../../lib/constants";
import rule from "../../../../lib/rules/no-undeclared-env-vars";
import { Project } from "../../../../lib/utils/calculate-inputs";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020, sourceType: "module" },
});

const cwd = path.join(__dirname, "../../../../__fixtures__/workspace-configs");
const webFilename = path.join(cwd, "/apps/web/index.js");

describe("Project reload functionality", () => {
  it("should reload workspace configurations when called", () => {
    const project = new Project(cwd);
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
    const project = new Project(cwd);
    const initialKey = project._key;
    const initialTest = project._test;

    // Call reload
    project.reload();

    // Verify that key and test configurations were regenerated
    expect(project._key).not.toBe(initialKey);
    expect(project._test).not.toBe(initialTest);
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
