import { setupTestFixtures } from "@turbo/test-utils";
import { transformer } from "../src/transforms/stabilize-ui";

describe("stabilize-ui", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "stabilize-ui",
  });

  it("adds no config where there was none", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "no-config",
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

  it("removes config if it was already enabled", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "enabled",
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
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 0,
          "deletions": 1,
        },
      }
    `);
  });

  it("renames config if disabled", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "disabled",
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
      ui: "stream",
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
