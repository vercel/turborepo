import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import { transformer } from "../src/transforms/update-schema-json-url";

describe("update-schema-url", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "update-schema-url",
  });

  it("updates schema URL from v1 to current version", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "v1-schema",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.v2.json",
      tasks: {
        build: {
          outputs: ["dist/**"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
        },
      }
    `);
  });

  it("does nothing if schema URL is already updated", () => {
    // load the fixture for the test
    const { root, read } = useFixture({
      fixture: "current-schema",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false },
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.com/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"],
        },
      },
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });
});
