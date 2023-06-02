import { transformer, fixGlobPattern } from "../src/transforms/clean-globs";
import { setupTestFixtures } from "@turbo/test-utils";
import getTransformerHelpers from "../src/utils/getTransformerHelpers";

describe("clean-globs", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "clean-globs",
  });

  test("basic", () => {
    // load the fixture for the test
    const { root, read, readJson } = useFixture({
      fixture: "clean-globs",
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "turbo.json": Object {
          "action": "modified",
          "additions": 6,
          "deletions": 6,
        },
      }
    `);
  });

  test("collapses back-to-back doublestars", () => {
    let badGlobPatterns = [
      ["../../app-store/**/**", "../../app-store/**"],
      ["**/**/result.json", "**/result.json"],
      ["**/**/**/**", "**"],
      ["**/foo/**/**/bar/**", "**/foo/**/bar/**"],
      ["**/foo/**/**/**/bar/**/**", "**/foo/**/bar/**"],
      ["**/foo/**/**/**/**/bar/**/**/**", "**/foo/**/bar/**"],
    ];

    const { log } = getTransformerHelpers({
      transformer: "test",
      rootPath: ".",
      options: { force: false, dry: false, print: false },
    });

    // Now let's test the function
    badGlobPatterns.forEach(([input, output]) => {
      expect(fixGlobPattern(input, log)).toBe(output);
    });
  });

  test("doesn't update valid globs and prints a message", () => {
    const { log } = getTransformerHelpers({
      transformer: "test",
      rootPath: ".",
      options: { force: false, dry: false, print: false },
    });

    // Now let's test the function
    expect(fixGlobPattern("a/b/c/*", log)).toBe("a/b/c/*");
  });

  test("transforms '!**/folder' to '**/[!folder]'", () => {
    let badGlobPatterns = [
      ["!**/dist", "**/[!dist]"],
      ["!**/node_modules", "**/[!node_modules]"],
      ["!**/foo/bar", "**/[!foo/bar]"],
      ["!**/foo/bar/baz", "**/[!foo/bar/baz]"],
      ["!**/foo/bar/baz/qux", "**/[!foo/bar/baz/qux]"],
    ];

    const { log } = getTransformerHelpers({
      transformer: "test",
      rootPath: ".",
      options: { force: false, dry: false, print: false },
    });

    // Now let's test the function
    badGlobPatterns.forEach(([input, output]) => {
      expect(fixGlobPattern(input, log)).toBe(output);
    });
  });

  test("transforms '**ext' to '**/*ext'", () => {
    let badGlobPatterns = [
      ["cypress/integration/**.test.ts", "cypress/integration/**/*.test.ts"],
      ["scripts/**.mjs", "scripts/**/*.mjs"],
      ["scripts/**.js", "scripts/**/*.js"],
      ["src/types/generated/**.ts", "src/types/generated/**/*.ts"],
      ["**md", "**/*md"],
      ["**txt", "**/*txt"],
      ["**html", "**/*html"],
    ];

    const { log } = getTransformerHelpers({
      transformer: "test",
      rootPath: ".",
      options: { force: false, dry: false, print: false },
    });

    // Now let's test the function
    badGlobPatterns.forEach(([input, output]) => {
      expect(fixGlobPattern(input, log)).toBe(output);
    });
  });

  test("transforms 'pre**' to pre*/**", () => {
    let badGlobPatterns = [
      ["pre**", "pre*/**"],
      ["pre**/foo", "pre*/**/foo"],
      ["pre**/foo/bar", "pre*/**/foo/bar"],
      ["pre**/foo/bar/baz", "pre*/**/foo/bar/baz"],
      ["pre**/foo/bar/baz/qux", "pre*/**/foo/bar/baz/qux"],
    ];

    const { log } = getTransformerHelpers({
      transformer: "test",
      rootPath: ".",
      options: { force: false, dry: false, print: false },
    });

    // Now let's test the function
    badGlobPatterns.forEach(([input, output]) => {
      expect(fixGlobPattern(input, log)).toBe(output);
    });
  });
});
