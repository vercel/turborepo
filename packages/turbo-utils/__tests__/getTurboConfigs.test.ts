import path from "node:path";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import JSON5 from "json5";
import type { TurboConfigs } from "../src/getTurboConfigs";
import { getTurboConfigs } from "../src/getTurboConfigs";

describe("getTurboConfigs", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    test: "common",
  });

  it("supports single-package repos", () => {
    const { root } = useFixture({ fixture: `single-package` });
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- We know it's in the fixture.
    const configs = getTurboConfigs(root)!;
    expect(configs).toHaveLength(1);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turborepo.com/schema.json",
        "globalEnv": [
          "UNORDERED",
          "CI",
        ],
        "tasks": {
          "build": {
            "dependsOn": [
              "^build",
            ],
          },
          "deploy": {
            "dependsOn": [
              "build",
              "test",
              "lint",
            ],
            "outputs": [],
          },
          "lint": {
            "outputs": [],
          },
          "test": {
            "dependsOn": [
              "build",
            ],
            "inputs": [
              "src/**/*.tsx",
              "src/**/*.ts",
              "test/**/*.ts",
              "test/**/*.tsx",
            ],
            "outputs": [],
          },
        },
      }
    `);
  });

  it("supports repos using workspace configs", () => {
    const { root } = useFixture({ fixture: `workspace-configs` });
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- We know it's in the fixture.
    const configs = getTurboConfigs(root)!;

    expect(configs).toHaveLength(3);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turborepo.com/schema.json",
        "globalEnv": [
          "CI",
        ],
        "tasks": {
          "build": {
            "env": [
              "ENV_1",
            ],
          },
        },
      }
    `);
    expect(configs[1].isRootConfig).toBe(false);
    expect(configs[1].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turborepo.com/schema.json",
        "extends": [
          "//",
        ],
        "tasks": {
          "build": {
            "env": [
              "ENV_2",
            ],
          },
        },
      }
    `);

    expect(configs[2].isRootConfig).toBe(false);
    expect(configs[2].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turborepo.com/schema.json",
        "extends": [
          "//",
        ],
        "tasks": {
          "build": {
            "env": [
              "IS_SERVER",
            ],
          },
        },
      }
    `);
  });

  it("supports repos with old workspace configuration format", () => {
    const { root } = useFixture({ fixture: `old-workspace-config` });
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- We know it's in the fixture.
    const configs = getTurboConfigs(root)!;

    expect(configs).toHaveLength(1);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turborepo.com/schema.json",
        "globalDependencies": [
          "**/.env.*local",
        ],
        "tasks": {
          "build": {
            "outputs": [
              ".next/**",
              "!.next/cache/**",
            ],
          },
          "dev": {
            "cache": false,
            "persistent": true,
          },
          "lint": {},
        },
      }
    `);
  });
});

// Test JSON5 parsing functionality directly
describe("JSON5 parsing for turbo.jsonc", () => {
  it("correctly parses turbo.jsonc with comments", () => {
    const turboJsoncContent = `{
      // This is a comment in turbo.jsonc
      "$schema": "https://turborepo.com/schema.json",
      "globalEnv": ["UNORDERED", "CI"], // Another comment
      "tasks": {
        "build": {
          // A workspace's build task depends on dependencies
          "dependsOn": ["^build"]
        },
        "test": {
          "dependsOn": ["build"],
          "outputs": [],
          "inputs": ["src/**/*.tsx", "src/**/*.ts", "test/**/*.ts", "test/**/*.tsx"]
        },
        "lint": {
          "outputs": []
        },
        "deploy": {
          "dependsOn": ["build", "test", "lint"],
          "outputs": []
        }
      }
    }`;

    const parsed: TurboConfigs = JSON5.parse(turboJsoncContent);

    expect(parsed).toMatchObject({
      $schema: "https://turborepo.com/schema.json",
      globalEnv: ["UNORDERED", "CI"],
      tasks: {
        build: {
          dependsOn: ["^build"],
        },
        test: {
          dependsOn: ["build"],
          outputs: [],
          inputs: [
            "src/**/*.tsx",
            "src/**/*.ts",
            "test/**/*.ts",
            "test/**/*.tsx",
          ],
        },
        lint: {
          outputs: [],
        },
        deploy: {
          dependsOn: ["build", "test", "lint"],
          outputs: [],
        },
      },
    });
  });
});
