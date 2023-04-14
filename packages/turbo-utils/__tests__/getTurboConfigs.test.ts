import path from "path";
import getTurboConfigs from "../src/getTurboConfigs";
import { setupTestFixtures } from "@turbo/test-utils";

describe("getTurboConfigs", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    test: "common",
  });

  it("single-package", async () => {
    const { root } = useFixture({ fixture: `single-package` });
    const configs = getTurboConfigs(root);
    expect(configs).toHaveLength(1);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      Object {
        "$schema": "https://turbo.build/schema.json",
        "globalEnv": Array [
          "UNORDERED",
          "CI",
        ],
        "pipeline": Object {
          "build": Object {
            "dependsOn": Array [
              "^build",
            ],
          },
          "deploy": Object {
            "dependsOn": Array [
              "build",
              "test",
              "lint",
            ],
            "outputs": Array [],
          },
          "lint": Object {
            "outputs": Array [],
          },
          "test": Object {
            "dependsOn": Array [
              "build",
            ],
            "inputs": Array [
              "src/**/*.tsx",
              "src/**/*.ts",
              "test/**/*.ts",
              "test/**/*.tsx",
            ],
            "outputs": Array [],
          },
        },
      }
    `);
  });

  it("workspace-configs", async () => {
    const { root } = useFixture({ fixture: `workspace-configs` });
    const configs = getTurboConfigs(root);

    expect(configs).toHaveLength(3);
    expect(configs[0].isRootConfig).toBe(true);
    expect(configs[0].config).toMatchInlineSnapshot(`
      Object {
        "$schema": "https://turbo.build/schema.json",
        "globalEnv": Array [
          "CI",
        ],
        "pipeline": Object {
          "build": Object {
            "env": Array [
              "ENV_1",
            ],
          },
        },
      }
    `);
    expect(configs[1].isRootConfig).toBe(false);
    expect(configs[1].config).toMatchInlineSnapshot(`
      Object {
        "$schema": "https://turbo.build/schema.json",
        "extends": Array [
          "//",
        ],
        "pipeline": Object {
          "build": Object {
            "env": Array [
              "ENV_2",
            ],
          },
        },
      }
    `);

    expect(configs[2].isRootConfig).toBe(false);
    expect(configs[2].config).toMatchInlineSnapshot(`
      Object {
        "$schema": "https://turbo.build/schema.json",
        "extends": Array [
          "//",
        ],
        "pipeline": Object {
          "build": Object {
            "env": Array [
              "IS_SERVER",
            ],
          },
        },
      }
    `);
  });
});
