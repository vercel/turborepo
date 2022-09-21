import merge from "deepmerge";
import {
  hasLegacyEnvVarDependencies,
  migratePipeline,
  migrateConfig,
} from "../src/transforms/migrate-env-var-dependencies";
import { TurboConfig } from "../src/types";

const getTestTurboConfig = (override: TurboConfig = {}): TurboConfig => {
  const config = {
    $schema: "./docs/public/schema.json",
    globalDependencies: ["$GLOBAL_ENV_KEY"],
    pipeline: {
      test: {
        outputs: ["coverage/**/*"],
        dependsOn: ["^build"],
      },
      lint: {
        outputs: [],
      },
      dev: {
        cache: false,
      },
      build: {
        outputs: ["dist/**/*", ".next/**/*"],
        dependsOn: ["^build", "$TASK_ENV_KEY", "$ANOTHER_ENV_KEY"],
      },
    },
  };

  return merge(config, override, {
    arrayMerge: (_, sourceArray) => sourceArray,
  });
};

describe("migrate-env-var-dependencies", () => {
  describe("hasLegacyEnvVarDependencies", () => {
    it("finds env keys in legacy turbo.json - has keys", async () => {
      const config = getTestTurboConfig();
      const { hasKeys, envVars } = hasLegacyEnvVarDependencies(config);
      expect(hasKeys).toEqual(true);
      expect(envVars).toMatchInlineSnapshot(`
              Array [
                "$GLOBAL_ENV_KEY",
                "$TASK_ENV_KEY",
                "$ANOTHER_ENV_KEY",
              ]
          `);
    });

    it("finds env keys in legacy turbo.json - multiple pipeline keys", async () => {
      const config = getTestTurboConfig({
        pipeline: { test: { dependsOn: ["$MY_ENV"] } },
      });
      const { hasKeys, envVars } = hasLegacyEnvVarDependencies(config);
      expect(hasKeys).toEqual(true);
      expect(envVars).toMatchInlineSnapshot(`
              Array [
                "$GLOBAL_ENV_KEY",
                "$MY_ENV",
                "$TASK_ENV_KEY",
                "$ANOTHER_ENV_KEY",
              ]
          `);
    });

    it("finds env keys in legacy turbo.json - no keys", async () => {
      // override to exclude keys
      const config = getTestTurboConfig({
        globalDependencies: [],
        pipeline: { build: { dependsOn: [] } },
      });
      const { hasKeys, envVars } = hasLegacyEnvVarDependencies(config);
      expect(hasKeys).toEqual(false);
      expect(envVars).toMatchInlineSnapshot(`Array []`);
    });
  });

  describe("migratePipeline", () => {
    it("migrates pipeline with env var dependencies", async () => {
      const config = getTestTurboConfig();
      const { build } = config?.pipeline ?? {};
      const pipeline = migratePipeline(build);
      expect(pipeline).toHaveProperty("env");
      expect(pipeline?.env).toMatchInlineSnapshot(`
        Array [
          "$TASK_ENV_KEY",
          "$ANOTHER_ENV_KEY",
        ]
      `);
      expect(pipeline?.dependsOn).toMatchInlineSnapshot(`
        Array [
          "^build",
        ]
      `);
    });

    it("migrates pipeline with no env var dependencies", async () => {
      const config = getTestTurboConfig();
      const { test } = config?.pipeline ?? {};
      const pipeline = migratePipeline(test);
      expect(pipeline.env).toBeUndefined();
      expect(pipeline?.dependsOn).toMatchInlineSnapshot(`
        Array [
          "^build",
        ]
      `);
    });

    it("migrates pipeline with existing env key", async () => {
      const config = getTestTurboConfig({
        pipeline: { test: { env: ["$MY_ENV"], dependsOn: ["^build"] } },
      });
      const { test } = config?.pipeline ?? {};
      const pipeline = migratePipeline(test);
      expect(pipeline).toHaveProperty("env");
      expect(pipeline?.env).toMatchInlineSnapshot(`
        Array [
          "$MY_ENV",
        ]
      `);
      expect(pipeline?.dependsOn).toMatchInlineSnapshot(`
        Array [
          "^build",
        ]
      `);
    });

    it("migrates pipeline with incomplete env key", async () => {
      const config = getTestTurboConfig({
        pipeline: {
          test: { env: ["$MY_ENV"], dependsOn: ["^build", "$SUPER_COOL"] },
        },
      });
      const { test } = config?.pipeline ?? {};
      const pipeline = migratePipeline(test);
      expect(pipeline).toHaveProperty("env");
      expect(pipeline?.env).toMatchInlineSnapshot(`
        Array [
          "$MY_ENV",
          "$SUPER_COOL",
        ]
      `);
      expect(pipeline?.dependsOn).toMatchInlineSnapshot(`
        Array [
          "^build",
        ]
      `);
    });

    it("migrates pipeline with duplicate env keys", async () => {
      const config = getTestTurboConfig({
        pipeline: {
          test: { env: ["$MY_ENV"], dependsOn: ["^build", "$MY_ENV"] },
        },
      });
      const { test } = config?.pipeline ?? {};
      const pipeline = migratePipeline(test);
      expect(pipeline).toHaveProperty("env");
      expect(pipeline?.env).toMatchInlineSnapshot(`
        Array [
          "$MY_ENV",
        ]
      `);
      expect(pipeline?.dependsOn).toMatchInlineSnapshot(`
        Array [
          "^build",
        ]
      `);
    });
  });

  describe("migrateConfig", () => {
    it("migrates config with env var dependencies", async () => {
      const config = getTestTurboConfig();
      const pipeline = migrateConfig(config);
      expect(pipeline).toMatchInlineSnapshot(`
        Object {
          "$schema": "./docs/public/schema.json",
          "env": Array [
            "$GLOBAL_ENV_KEY",
          ],
          "globalDependencies": Array [],
          "pipeline": Object {
            "build": Object {
              "dependsOn": Array [
                "^build",
              ],
              "env": Array [
                "$TASK_ENV_KEY",
                "$ANOTHER_ENV_KEY",
              ],
              "outputs": Array [
                "dist/**/*",
                ".next/**/*",
              ],
            },
            "dev": Object {
              "cache": false,
            },
            "lint": Object {
              "outputs": Array [],
            },
            "test": Object {
              "dependsOn": Array [
                "^build",
              ],
              "outputs": Array [
                "coverage/**/*",
              ],
            },
          },
        }
      `);
    });

    it("migrates config with no env var dependencies", async () => {
      const config = getTestTurboConfig({
        globalDependencies: [],
        pipeline: {
          build: { dependsOn: ["^build"] },
        },
      });
      const pipeline = migrateConfig(config);
      expect(pipeline).toMatchInlineSnapshot(`
        Object {
          "$schema": "./docs/public/schema.json",
          "globalDependencies": Array [],
          "pipeline": Object {
            "build": Object {
              "dependsOn": Array [
                "^build",
              ],
              "outputs": Array [
                "dist/**/*",
                ".next/**/*",
              ],
            },
            "dev": Object {
              "cache": false,
            },
            "lint": Object {
              "outputs": Array [],
            },
            "test": Object {
              "dependsOn": Array [
                "^build",
              ],
              "outputs": Array [
                "coverage/**/*",
              ],
            },
          },
        }
      `);
    });

    it("migrates config with inconsistent config", async () => {
      const config = getTestTurboConfig({
        pipeline: {
          test: { env: ["$MY_ENV"], dependsOn: ["^build", "$SUPER_COOL"] },
        },
      });
      const pipeline = migrateConfig(config);
      expect(pipeline).toMatchInlineSnapshot(`
        Object {
          "$schema": "./docs/public/schema.json",
          "env": Array [
            "$GLOBAL_ENV_KEY",
          ],
          "globalDependencies": Array [],
          "pipeline": Object {
            "build": Object {
              "dependsOn": Array [
                "^build",
              ],
              "env": Array [
                "$TASK_ENV_KEY",
                "$ANOTHER_ENV_KEY",
              ],
              "outputs": Array [
                "dist/**/*",
                ".next/**/*",
              ],
            },
            "dev": Object {
              "cache": false,
            },
            "lint": Object {
              "outputs": Array [],
            },
            "test": Object {
              "dependsOn": Array [
                "^build",
              ],
              "env": Array [
                "$MY_ENV",
                "$SUPER_COOL",
              ],
              "outputs": Array [
                "coverage/**/*",
              ],
            },
          },
        }
      `);
    });

    it("migrates config with duplicate env keys", async () => {
      const config = getTestTurboConfig({
        pipeline: {
          test: { env: ["$MY_ENV"], dependsOn: ["^build", "$MY_ENV"] },
        },
      });
      const pipeline = migrateConfig(config);
      expect(pipeline).toMatchInlineSnapshot(`
        Object {
          "$schema": "./docs/public/schema.json",
          "env": Array [
            "$GLOBAL_ENV_KEY",
          ],
          "globalDependencies": Array [],
          "pipeline": Object {
            "build": Object {
              "dependsOn": Array [
                "^build",
              ],
              "env": Array [
                "$TASK_ENV_KEY",
                "$ANOTHER_ENV_KEY",
              ],
              "outputs": Array [
                "dist/**/*",
                ".next/**/*",
              ],
            },
            "dev": Object {
              "cache": false,
            },
            "lint": Object {
              "outputs": Array [],
            },
            "test": Object {
              "dependsOn": Array [
                "^build",
              ],
              "env": Array [
                "$MY_ENV",
              ],
              "outputs": Array [
                "coverage/**/*",
              ],
            },
          },
        }
      `);
    });
  });
});
