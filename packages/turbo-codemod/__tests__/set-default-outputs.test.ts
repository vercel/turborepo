import { transformer } from "../src/transforms/set-default-outputs";
import { setupTestFixtures } from "@turbo/test-utils";

describe("set-default-outputs", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "set-default-outputs",
  });
  it("migrates turbo.json outputs - basic", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "old-outputs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        "build-one": {
          outputs: ["foo"],
        },
        "build-two": {},
        "build-three": {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 2,
          "deletions": 1,
        },
      }
    `);
  });

  it("migrates turbo.json outputs - workspace configs", async () => {
    // load the fixture for the test
    const { root, readJson } = useFixture({
      fixture: "workspace-configs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(readJson("turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        "build-one": {
          outputs: ["foo"],
        },
        "build-two": {},
        "build-three": {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(readJson("apps/docs/turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      pipeline: {
        build: {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(readJson("apps/web/turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      pipeline: {
        build: {},
      },
    });

    expect(readJson("packages/ui/turbo.json") || "{}").toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      pipeline: {
        "build-three": {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "apps/docs/turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
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
          "deletions": 1,
        },
      }
    `);
  });

  it("migrates turbo.json outputs - dry", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "old-outputs",
    });

    const turboJson = JSON.parse(read("turbo.json") || "{}");

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: true, print: false },
    });

    // make sure it didn't change
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboJson);

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "skipped",
          "additions": 2,
          "deletions": 1,
        },
      }
    `);
  });

  it("migrates turbo.json outputs - print", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "old-outputs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: true },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        "build-one": {
          outputs: ["foo"],
        },
        "build-two": {},
        "build-three": {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 2,
          "deletions": 1,
        },
      }
    `);
  });

  it("migrates turbo.json outputs - dry & print", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "old-outputs",
    });

    const turboJson = JSON.parse(read("turbo.json") || "{}");

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: true, print: false },
    });

    // make sure it didn't change
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboJson);

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "skipped",
          "additions": 2,
          "deletions": 1,
        },
      }
    `);
  });

  it("migrates turbo.json outputs - invalid", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "invalid-outputs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        "build-one": {
          outputs: ["foo"],
        },
        "build-two": {},
        "build-three": {
          outputs: ["dist/**", "build/**"],
        },
        "garbage-in-numeric-0": {
          outputs: ["dist/**", "build/**"],
        },
        "garbage-in-numeric": {
          outputs: 42,
        },
        "garbage-in-string": {
          outputs: "string",
        },
        "garbage-in-empty-string": {
          outputs: ["dist/**", "build/**"],
        },
        "garbage-in-null": {
          outputs: ["dist/**", "build/**"],
        },
        "garbage-in-false": {
          outputs: ["dist/**", "build/**"],
        },
        "garbage-in-true": {
          outputs: true,
        },
        "garbage-in-object": {
          outputs: {},
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 6,
          "deletions": 5,
        },
      }
    `);
  });

  it("migrates turbo.json outputs - config with no pipeline", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-pipeline",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      globalDependencies: ["$NEXT_PUBLIC_API_KEY", "$STRIPE_API_KEY", ".env"],
      pipeline: {},
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

  it("migrates turbo.json outputs - config with no outputs", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-outputs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        "build-one": {
          dependsOn: ["build-two"],
          outputs: ["dist/**", "build/**"],
        },
        "build-two": {
          cache: false,
        },
        "build-three": {
          persistent: true,
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 2,
          "deletions": 0,
        },
      }
    `);
  });

  it("errors if no turbo.json can be found", async () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-turbo-json",
    });

    expect(read("turbo.json")).toBeUndefined();

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(read("turbo.json")).toBeUndefined();
    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toMatch(
      /No turbo\.json found at .*?\. Is the path correct\?/
    );
  });

  it("errors if package.json config exists and has not been migrated", async () => {
    // load the fixture for the test
    const { root } = useFixture({
      fixture: "old-config",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toMatch(
      'turbo" key detected in package.json. Run `npx @turbo/codemod transform create-turbo-config` first'
    );
  });
});
