import { setupTestFixtures } from "@turbo/test-utils";
import { transformer } from "../src/transforms/rename-pipeline";

describe("rename-pipeline", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "rename-pipeline",
  });

  it("migrates turbo.json pipeline - root config only", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "root-only",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      globalDependencies: ["important.txt"],
      tasks: {
        build: {
          outputs: ["dist"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
        },
      }
    `);
  });

  it("migrates turbo.json pipeline - workspace configs", () => {
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
      $schema: "https://turbo.build/schema.json",
      tasks: {
        build: {
          dependsOn: ["^build"],
          outputs: [".next/**", "!.next/cache/**"],
        },
        dev: {
          cache: false,
        },
        lint: {
          outputs: [],
        },
        test: {
          outputs: [],
        },
      },
    });

    expect(JSON.parse(read("apps/web/turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      tasks: {
        build: {
          dependsOn: [],
        },
      },
    });

    expect(JSON.parse(read("packages/ui/turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      extends: ["//"],
      tasks: {
        test: {
          dependsOn: ["build"],
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
          "deletions": 1,
        },
        "packages/ui/turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
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

  it("does not do anything if there is already a top level tasks key", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "with-tasks",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      tasks: {
        build: {
          outputs: ["dist"],
        },
      },
    });
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });
});
