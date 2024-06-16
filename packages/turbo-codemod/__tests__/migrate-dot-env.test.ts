import { setupTestFixtures } from "@turbo/test-utils";
import { type Schema } from "@turbo/types";
import { transformer } from "../src/transforms/migrate-dot-env";

describe("migrate-dot-env", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "migrate-dot-env",
  });
  it("migrates turbo.json dot-env - basic", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "with-dot-env",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      globalDependencies: [".env"],
      tasks: {
        "build-one": {
          inputs: ["$TURBO_DEFAULT$", "build-one/.env"],
        },
        "build-two": {
          inputs: ["build-two/main.js", "build-two/.env"],
        },
        "build-three": {},
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 3,
          "deletions": 3,
        },
      }
    `);
  });

  it("migrates turbo.json dot-env - workspace configs", () => {
    // load the fixture for the test
    const { root, readJson } = useFixture({
      fixture: "workspace-configs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(readJson("turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      tasks: {
        "build-one": {
          inputs: ["$TURBO_DEFAULT$", "build-one/.env"],
        },
        "build-two": {
          inputs: ["build-two/**/*.ts", "build-two/.env"],
        },
        "build-three": {},
      },
    });

    expect(readJson("apps/docs/turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      tasks: {
        build: {},
      },
    });

    expect(readJson("apps/web/turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      tasks: {
        build: {
          inputs: ["src/**/*.ts", ".env"],
        },
      },
    });

    expect(readJson("packages/ui/turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      tasks: {
        "build-three": {
          inputs: ["$TURBO_DEFAULT$", ".env"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "apps/docs/turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
        "apps/web/turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
        "packages/ui/turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "modified",
          "additions": 2,
          "deletions": 2,
        },
      }
    `);
  });

  it("migrates turbo.json dot-env - dry", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "with-dot-env",
    });

    const turboJson = JSON.parse(read("turbo.json") || "{}") as Schema;

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: true, print: false },
    });

    // make sure it didn't change
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboJson);

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "skipped",
          "additions": 3,
          "deletions": 3,
        },
      }
    `);
  });

  it("migrates turbo.json dot-env - print", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "with-dot-env",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: true },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      globalDependencies: [".env"],
      tasks: {
        "build-one": {
          inputs: ["$TURBO_DEFAULT$", "build-one/.env"],
        },
        "build-three": {},
        "build-two": {
          inputs: ["build-two/main.js", "build-two/.env"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 3,
          "deletions": 3,
        },
      }
    `);
  });

  it("migrates turbo.json dot-env - dry & print", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "with-dot-env",
    });

    const turboJson = JSON.parse(read("turbo.json") || "{}") as Schema;

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: true, print: false },
    });

    // make sure it didn't change
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboJson);

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "skipped",
          "additions": 3,
          "deletions": 3,
        },
      }
    `);
  });

  it("migrates turbo.json dot-env - config with no pipeline", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-pipeline",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      globalDependencies: ["$NEXT_PUBLIC_API_KEY", "$STRIPE_API_KEY", ".env"],
      tasks: {},
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  it("migrates turbo.json dot-env - config with no dot env", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-dot-env",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      tasks: {
        "build-one": {
          dependsOn: ["build-two"],
        },
        "build-two": {
          cache: false,
        },
        "build-three": {
          persistent: true,
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
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
});
