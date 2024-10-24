import path from "node:path";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import { getTurboConfigs } from "../src/getTurboConfigs";

describe("getTurboConfigs", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    test: "common",
  });

  it("supports single-package repos", () => {
    const { root } = useFixture({ fixture: `single-package` });
    const configs = getTurboConfigs(root);
    expect(configs).toHaveLength(1);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turbo.build/schema.json",
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
    const configs = getTurboConfigs(root);

    expect(configs).toHaveLength(3);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turbo.build/schema.json",
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
        "$schema": "https://turbo.build/schema.json",
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
        "$schema": "https://turbo.build/schema.json",
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
    const configs = getTurboConfigs(root);

    expect(configs).toHaveLength(1);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      {
        "$schema": "https://turbo.build/schema.json",
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
