import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import type { SchemaV2 } from "@turbo/types";
import {
  transformer,
  migrateRootConfig,
  migrateTaskConfigs,
} from "../src/transforms/stabilize-env-mode";

describe("stabilize-env-mode", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "stabilize-env-mode",
  });

  it("skips migrateRootConfig when no pipeline key", () => {
    const config: SchemaV2 = {
      $schema: "./docs/public/schema.json",
      globalDependencies: ["$GLOBAL_ENV_KEY"],
      tasks: {
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
          outputs: ["dist/**/*", ".next/**/*", "!.next/cache/**"],
          dependsOn: ["^build", "$TASK_ENV_KEY", "$ANOTHER_ENV_KEY"],
        },
      },
    };

    // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-argument -- Testing a situation outside of types that users can get themselves into at runtime
    const doneConfig = migrateRootConfig(config as any);

    expect(doneConfig).toEqual(config);
  });

  it("skips migrateTaskConfigs when no pipeline key", () => {
    const config: SchemaV2 = {
      $schema: "./docs/public/schema.json",
      globalDependencies: ["$GLOBAL_ENV_KEY"],
      tasks: {
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
          outputs: ["dist/**/*", ".next/**/*", "!.next/cache/**"],
          dependsOn: ["^build", "$TASK_ENV_KEY", "$ANOTHER_ENV_KEY"],
        },
      },
    };

    // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-argument -- Testing a situation outside of types that users can get themselves into at runtime
    const doneConfig = migrateTaskConfigs(config as any);

    expect(doneConfig).toEqual(config);
  });

  it("migrates env-mode has-both", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "has-both",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      globalPassThroughEnv: [
        "EXPERIMENTAL_GLOBAL_PASSTHROUGH",
        "GLOBAL_PASSTHROUGH",
      ],
      pipeline: {
        build: {
          passThroughEnv: ["EXPERIMENTAL_TASK_PASSTHROUGH", "TASK_PASSTHROUGH"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "modified",
          "additions": 2,
          "deletions": 4,
        },
      }
    `);
  });

  it("migrates env-mode has-duplicates", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "has-duplicates",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      globalPassThroughEnv: [
        "DUPLICATE_GLOBAL",
        "EXPERIMENTAL_GLOBAL_PASSTHROUGH",
        "GLOBAL_PASSTHROUGH",
      ],
      pipeline: {
        build: {
          passThroughEnv: [
            "DUPLICATE_TASK",
            "EXPERIMENTAL_TASK_PASSTHROUGH",
            "TASK_PASSTHROUGH",
          ],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "modified",
          "additions": 2,
          "deletions": 6,
        },
      }
    `);
  });

  it("migrates env-mode has-empty", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "has-empty",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      globalPassThroughEnv: [],
      pipeline: {
        build: {
          passThroughEnv: [],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "modified",
          "additions": 2,
          "deletions": 2,
        },
      }
    `);
  });

  it("migrates env-mode has-neither", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "has-neither",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      pipeline: {
        build: {},
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  it("migrates env-mode has-new", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "has-new",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      globalPassThroughEnv: ["GLOBAL_PASSTHROUGH"],
      pipeline: {
        build: {
          passThroughEnv: ["TASK_PASSTHROUGH"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  it("migrates env-mode has-old", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "has-old",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      globalPassThroughEnv: ["GLOBAL_PASSTHROUGH"],
      pipeline: {
        build: {
          passThroughEnv: ["TASK_PASSTHROUGH"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "modified",
          "additions": 2,
          "deletions": 2,
        },
      }
    `);
  });

  it("migrates env-mode workspace-configs", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "workspace-configs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      globalPassThroughEnv: [
        "EXPERIMENTAL_GLOBAL_PASSTHROUGH",
        "GLOBAL_PASSTHROUGH",
      ],
      pipeline: {
        build: {
          passThroughEnv: ["EXPERIMENTAL_TASK_PASSTHROUGH", "TASK_PASSTHROUGH"],
        },
      },
    });

    expect(JSON.parse(read("apps/docs/turbo.json") || "{}")).toStrictEqual({
      extends: ["//"],
      pipeline: {
        build: {
          passThroughEnv: [
            "DOCS_TASK_PASSTHROUGH",
            "EXPERIMENTAL_DOCS_TASK_PASSTHROUGH",
          ],
        },
      },
    });

    expect(JSON.parse(read("apps/website/turbo.json") || "{}")).toStrictEqual({
      extends: ["//"],
      pipeline: {
        build: {
          passThroughEnv: [
            "EXPERIMENTAL_WEBSITE_TASK_PASSTHROUGH",
            "WEBSITE_TASK_PASSTHROUGH",
          ],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "apps/docs/turbo.json": {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
        },
        "apps/website/turbo.json": {
          "action": "modified",
          "additions": 1,
          "deletions": 2,
        },
        "turbo.json": {
          "action": "modified",
          "additions": 2,
          "deletions": 4,
        },
      }
    `);
  });

  it("errors if no turbo.json can be found", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-turbo-json",
    });

    expect(read("turbo.json")).toBeUndefined();

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(read("turbo.json")).toBeUndefined();
    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toMatch(
      /No turbo\.json found at .*?\. Is the path correct\?/
    );
  });

  it("errors if package.json config exists and has not been migrated", () => {
    // load the fixture for the test
    const { root } = useFixture({
      fixture: "old-config",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toMatch(
      'turbo" key detected in package.json. Run `npx @turbo/codemod transform create-turbo-config` first'
    );
  });
});
