import path from "node:path";
import fs from "node:fs";
import { Linter } from "eslint";
import { describe, expect, it, beforeAll, afterAll } from "@jest/globals";
import type { SchemaV1 } from "@turbo/types";
import rule, { clearCache } from "../../../lib/rules/no-undeclared-env-vars";

const cwd = path.join(__dirname, "../../../__fixtures__/workspace-configs");
const webFilename = path.join(cwd, "/apps/web/index.js");
const docsFilename = path.join(cwd, "/apps/docs/index.js");

// Known good turbo.json state
const KNOWN_GOOD_TURBO_JSON = {
  $schema: "https://turborepo.com/schema.json",
  globalEnv: ["CI"],
  globalDotEnv: [".env", "missing.env"],
  pipeline: {
    build: {
      env: ["ENV_1"],
    },
  },
};

describe("Hash-based change detection", () => {
  const turboJsonPath = path.join(cwd, "turbo.json");
  let originalTurboJson: string;

  beforeAll(() => {
    // Save whatever state exists
    try {
      originalTurboJson = fs.readFileSync(turboJsonPath, "utf8");
    } catch {
      originalTurboJson = JSON.stringify(KNOWN_GOOD_TURBO_JSON, null, 2);
    }

    // Start with known good state
    fs.writeFileSync(
      turboJsonPath,
      JSON.stringify(KNOWN_GOOD_TURBO_JSON, null, 2)
    );
    clearCache();
  });

  afterAll(() => {
    // Restore original state
    fs.writeFileSync(turboJsonPath, originalTurboJson);
    clearCache();
  });

  it("should detect turbo.json changes between file lints", () => {
    const linter = new Linter();
    linter.defineRule("turbo/no-undeclared-env-vars", rule);

    const backup = fs.readFileSync(turboJsonPath, "utf8");
    try {
      // First lint - ENV_3 should fail because it's not in turbo.json
      const firstResults = linter.verify(
        "const { ENV_3 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(firstResults).toHaveLength(1);
      expect(firstResults[0].message).toContain("ENV_3 is not listed");

      // Modify turbo.json to add ENV_3
      const modifiedConfig = {
        ...KNOWN_GOOD_TURBO_JSON,
        globalEnv: ["ENV_3"],
      };
      fs.writeFileSync(turboJsonPath, JSON.stringify(modifiedConfig, null, 2));

      // Second lint - ENV_3 should now pass because hash detection picks up the change
      const secondResults = linter.verify(
        "const { ENV_3 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(secondResults).toHaveLength(0);
    } finally {
      fs.writeFileSync(turboJsonPath, backup);
      clearCache();
    }
  });

  it("should cache project when turbo.json hasn't changed", () => {
    // Lint the same code multiple times without changing turbo.json
    for (let i = 0; i < 5; i++) {
      const results = linter.verify(
        "const { ENV_2 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      // Should always pass since ENV_2 is valid and nothing changed
      expect(results).toHaveLength(0);
    }
  });

  it("should efficiently lint multiple files without reloading", () => {
    // Lint web files (ENV_2 is valid in apps/web)
    for (let i = 0; i < 3; i++) {
      const results = linter.verify(
        "const { ENV_2 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(results).toHaveLength(0);
    }

    // Lint docs files (ENV_3 is valid in docs workspace)
    for (let i = 0; i < 3; i++) {
      const results = linter.verify(
        "const { ENV_3 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: docsFilename }
      );

      expect(results).toHaveLength(0);
    }
  });

  it("should detect changes even after multiple unchanged lints", () => {
    // Lint several files without changes
    for (let i = 0; i < 5; i++) {
      const results = linter.verify(
        "const { ENV_2 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(results).toHaveLength(0);
    }

    // Now modify turbo.json
    const modifiedConfig: SchemaV1 = {
      ...(JSON.parse(originalTurboJson) as SchemaV1),
      globalEnv: ["ENV_3", "ENV_4"],
    };
    fs.writeFileSync(turboJsonPath, JSON.stringify(modifiedConfig, null, 2));

    // Next lint should detect the change
    const results = linter.verify(
      "const { ENV_3, ENV_4 } = process.env;",
      {
        rules: {
          "turbo/no-undeclared-env-vars": ["error", { cwd }],
        },
        parserOptions: { ecmaVersion: 2020 },
      },
      { filename: webFilename }
    );

    expect(results).toHaveLength(0);
  });

  it("should handle whitespace-only changes correctly", () => {
    // First lint
    const firstResults = linter.verify(
      "const { ENV_2 } = process.env;",
      {
        rules: {
          "turbo/no-undeclared-env-vars": ["error", { cwd }],
        },
        parserOptions: { ecmaVersion: 2020 },
      },
      { filename: webFilename }
    );

    expect(firstResults).toHaveLength(0);

    // Modify turbo.json with only whitespace changes
    const config = JSON.parse(originalTurboJson) as SchemaV1;
    const whitespaceChanged = JSON.stringify(config, null, 4); // Different indentation
    fs.writeFileSync(turboJsonPath, whitespaceChanged);

    // Should still work correctly (whitespace doesn't affect functionality)
    const secondResults = linter.verify(
      "const { ENV_2 } = process.env;",
      {
        rules: {
          "turbo/no-undeclared-env-vars": ["error", { cwd }],
        },
        parserOptions: { ecmaVersion: 2020 },
      },
      { filename: webFilename }
    );

    expect(secondResults).toHaveLength(0);
  });

  it("should detect changes across different workspace turbo.json files", () => {
    const webTurboJsonPath = path.join(cwd, "apps/web/turbo.json");
    let originalWebTurboJson: string | null = null;

    try {
      originalWebTurboJson = fs.readFileSync(webTurboJsonPath, "utf8");
    } catch {
      // File might not exist
    }

    try {
      // First lint - WEB_CUSTOM_VAR should fail
      const firstResults = linter.verify(
        "const { WEB_CUSTOM_VAR } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(firstResults).toHaveLength(1);

      // Create/modify workspace turbo.json
      const webConfig: SchemaV1 = {
        extends: ["//"],
        pipeline: {
          build: {
            env: ["WEB_CUSTOM_VAR"],
          },
        },
      };
      fs.writeFileSync(webTurboJsonPath, JSON.stringify(webConfig, null, 2));

      // Should detect the workspace config change
      const secondResults = linter.verify(
        "const { WEB_CUSTOM_VAR } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(secondResults).toHaveLength(0);
    } finally {
      // Cleanup
      if (originalWebTurboJson) {
        fs.writeFileSync(webTurboJsonPath, originalWebTurboJson);
      } else {
        try {
          fs.unlinkSync(webTurboJsonPath);
        } catch {
          // File might not exist
        }
      }
    }
  });

  it("should detect content changes through multiple modifications", () => {
    const backup = fs.readFileSync(turboJsonPath, "utf8");

    try {
      // First lint with original config (ENV_2 is valid in apps/web)
      const firstResults = linter.verify(
        "const { ENV_2 } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(firstResults).toHaveLength(0);

      // Modify config to add NEW_VAR
      const modifiedConfig = {
        ...(JSON.parse(backup) as Record<string, unknown>),
        globalEnv: ["CI", "NEW_VAR"],
      };
      fs.writeFileSync(turboJsonPath, JSON.stringify(modifiedConfig, null, 2));

      // NEW_VAR should now be valid
      const secondResults = linter.verify(
        "const { NEW_VAR } = process.env;",
        {
          rules: {
            "turbo/no-undeclared-env-vars": ["error", { cwd }],
          },
          parserOptions: { ecmaVersion: 2020 },
        },
        { filename: webFilename }
      );

      expect(secondResults).toHaveLength(0);
    } finally {
      // Always restore, even if test fails
      fs.writeFileSync(turboJsonPath, backup);
    }
  });
});
