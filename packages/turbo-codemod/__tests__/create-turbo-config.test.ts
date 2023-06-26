import { transformer } from "../src/transforms/create-turbo-config";
import { setupTestFixtures } from "@turbo/test-utils";
import fs from "fs-extra";

describe("create-turbo-config", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "create-turbo-config",
  });

  test("package.json config exists but no turbo.json config - basic", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-turbo-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // turbo.json should now exist (and match the package.json config)
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboConfig);

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 0,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);
  });

  test("package.json config exists but no turbo.json config - repeat run", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-turbo-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // turbo.json should now exist (and match the package.json config)
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboConfig);

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 0,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);

    // run the transformer
    const repeatResult = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });
    // result should be correct
    expect(repeatResult.fatalError).toBeUndefined();
    expect(repeatResult.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
        "turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  test("package.json config exists but no turbo.json config - dry", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-turbo-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: true, print: false },
    });

    // turbo.json still not exist (dry run)
    expect(read("turbo.json")).toBeUndefined();

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "skipped",
          "additions": 0,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "skipped",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);
  });

  test("package.json config exists but no turbo.json config - print", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-turbo-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: true },
    });

    // turbo.json should now exist (and match the package.json config)
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboConfig);

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 0,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);
  });

  test("package.json config exists but no turbo.json config - dry & print", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-turbo-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: true, print: true },
    });

    // turbo.json still not exist (dry run)
    expect(read("turbo.json")).toBeUndefined();

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "skipped",
          "additions": 0,
          "deletions": 1,
        },
        "turbo.json": Object {
          "action": "skipped",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);
  });

  test("no package.json config or turbo.json file exists", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const packageJsonConfig = JSON.parse(read("package.json") || "{}");
    const turboConfig = packageJsonConfig.turbo;
    expect(turboConfig).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // turbo.json should still not exist
    expect(read("turbo.json")).toBeUndefined();

    // make sure we didn't change the package.json
    expect(JSON.parse(read("package.json") || "{}")).toEqual(packageJsonConfig);

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
        "turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  test("no package.json file exists", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-json-file" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // turbo.json should still not exist
    expect(read("turbo.json")).toBeUndefined();

    // result should be correct
    expect(result.fatalError?.message).toMatch(
      /No package\.json found at .*?\. Is the path correct\?/
    );
  });

  test("turbo.json file exists and no package.json config exists", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "turbo-json-config" });

    // turbo.json should exist
    expect(read("turbo.json")).toBeDefined();

    // no config should exist in package.json
    const packageJsonConfig = JSON.parse(read("package.json") || "{}");
    const turboConfig = packageJsonConfig.turbo;
    expect(turboConfig).toBeUndefined();

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // turbo.json should still exist
    expect(read("turbo.json")).toBeDefined();

    // make sure we didn't change the package.json
    expect(JSON.parse(read("package.json") || "{}")).toEqual(packageJsonConfig);

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
        "turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  test("turbo.json file exists and package.json config exists", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "both-configs" });

    // turbo.json should exist
    const turboJsonConfig = JSON.parse(read("turbo.json") || "{}");
    expect(turboJsonConfig.pipeline).toBeDefined();

    // no config should exist in package.json
    const packageJsonConfig = JSON.parse(read("package.json") || "{}");
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // make sure we didn't change the package.json
    expect(JSON.parse(read("package.json") || "{}")).toEqual(packageJsonConfig);

    // make sure we didn't change the turbo.json
    expect(JSON.parse(read("turbo.json") || "{}")).toEqual(turboJsonConfig);

    // result should be correct
    expect(result.fatalError?.message).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
        "turbo.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);
  });

  test("errors when unable to write json", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-turbo-json-config" });

    // turbo.json should not exist
    expect(read("turbo.json")).toBeUndefined();

    // get config from package.json for comparison later
    const turboConfig = JSON.parse(read("package.json") || "{}").turbo;
    expect(turboConfig).toBeDefined();

    const mockWriteJsonSync = jest
      .spyOn(fs, "writeJsonSync")
      .mockImplementation(() => {
        throw new Error("could not write file");
      });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // turbo.json should still not exist (error writing)
    expect(read("turbo.json")).toBeUndefined();

    // result should be correct
    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toMatch(
      "Encountered an error while transforming files"
    );
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "error",
          "additions": 0,
          "deletions": 1,
          "error": [Error: could not write file],
        },
        "turbo.json": Object {
          "action": "error",
          "additions": 1,
          "deletions": 0,
          "error": [Error: could not write file],
        },
      }
    `);

    mockWriteJsonSync.mockRestore();
  });
});
