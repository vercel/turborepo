import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import { transformer, fixGlobPattern } from "../src/transforms/clean-globs";

describe("clean-globs", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "clean-globs"
  });

  it("basic", () => {
    // load the fixture for the test
    const { root } = useFixture({
      fixture: "clean-globs"
    });

    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false }
    });

    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      {
        "turbo.json": {
          "action": "modified",
          "additions": 6,
          "deletions": 6,
        },
      }
    `);
  });

  it("handles missing pipeline key without crashing", () => {
    const { root } = useFixture({
      fixture: "no-pipeline"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false }
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

  it("collapses back-to-back doublestars", () => {
    const badGlobPatterns = [
      ["../../app-store/**/**", "../../app-store/**"],
      ["**/**/result.json", "**/result.json"],
      ["**/**/**/**", "**"],
      ["**/foo/**/**/bar/**", "**/foo/**/bar/**"],
      ["**/foo/**/**/**/bar/**/**", "**/foo/**/bar/**"],
      ["**/foo/**/**/**/**/bar/**/**/**", "**/foo/**/bar/**"]
    ];

    // Now let's test the function
    for (const [input, output] of badGlobPatterns) {
      expect(fixGlobPattern(input)).toBe(output);
    }
  });

  it("doesn't update valid globs and prints a message", () => {
    // Now let's test the function
    expect(fixGlobPattern("a/b/c/*")).toBe("a/b/c/*");
  });

  it("transforms '**ext' to '**/*ext'", () => {
    const badGlobPatterns = [
      ["cypress/integration/**.test.ts", "cypress/integration/**/*.test.ts"],
      ["scripts/**.mjs", "scripts/**/*.mjs"],
      ["scripts/**.js", "scripts/**/*.js"],
      ["src/types/generated/**.ts", "src/types/generated/**/*.ts"],
      ["**md", "**/*md"],
      ["**txt", "**/*txt"],
      ["**html", "**/*html"]
    ];

    // Now let's test the function
    for (const [input, output] of badGlobPatterns) {
      expect(fixGlobPattern(input)).toBe(output);
    }
  });

  it("transforms 'pre**' to pre*/**", () => {
    const badGlobPatterns = [
      ["pre**", "pre*/**"],
      ["pre**/foo", "pre*/**/foo"],
      ["pre**/foo/bar", "pre*/**/foo/bar"],
      ["pre**/foo/bar/baz", "pre*/**/foo/bar/baz"],
      ["pre**/foo/bar/baz/qux", "pre*/**/foo/bar/baz/qux"]
    ];

    // Now let's test the function
    for (const [input, output] of badGlobPatterns) {
      expect(fixGlobPattern(input)).toBe(output);
    }
  });

  it("should collapse back-to-back doublestars to a single doublestar", () => {
    expect(fixGlobPattern("../../app-store/**/**")).toBe("../../app-store/**");
    expect(fixGlobPattern("**/**/result.json")).toBe("**/result.json");
  });

  it("should change **.ext to **/*.ext", () => {
    expect(fixGlobPattern("**.js")).toBe("**/*.js");
    expect(fixGlobPattern("**.json")).toBe("**/*.json");
    expect(fixGlobPattern("**.ext")).toBe("**/*.ext");
  });

  it("should change prefix** to prefix*/**", () => {
    expect(fixGlobPattern("app**")).toBe("app*/**");
    expect(fixGlobPattern("src**")).toBe("src*/**");
    expect(fixGlobPattern("prefix**")).toBe("prefix*/**");
  });

  it("should collapse back-to-back doublestars and change **.ext to **/*.ext", () => {
    expect(fixGlobPattern("../../app-store/**/**/*.js")).toBe(
      "../../app-store/**/*.js"
    );
    expect(fixGlobPattern("**/**/result.json")).toBe("**/result.json");
  });

  it("should collapse back-to-back doublestars and change prefix** to prefix*/**", () => {
    expect(fixGlobPattern("../../app-store/**/**prefix**")).toBe(
      "../../app-store/**/*prefix*/**"
    );
    expect(fixGlobPattern("**/**/prefix**")).toBe("**/prefix*/**");
  });

  it("should not modify valid glob patterns", () => {
    expect(fixGlobPattern("src/**/*.js")).toBe("src/**/*.js");
    expect(fixGlobPattern("src/**/test/*.js")).toBe("src/**/test/*.js");
    expect(fixGlobPattern("src/**/test/**/*.js")).toBe("src/**/test/**/*.js");
    expect(fixGlobPattern("src/**/test/**/result.json")).toBe(
      "src/**/test/**/result.json"
    );
  });

  it("should handle glob patterns with non-ASCII characters", () => {
    expect(fixGlobPattern("src/日本語/**/*.js")).toBe("src/日本語/**/*.js");
    expect(fixGlobPattern("src/中文/**/*.json")).toBe("src/中文/**/*.json");
    expect(fixGlobPattern("src/русский/**/*.ts")).toBe("src/русский/**/*.ts");
  });

  it("should handle glob patterns with emojis", () => {
    expect(fixGlobPattern("src/👋**/*.js")).toBe("src/👋*/**/*.js");
    expect(fixGlobPattern("src/🌎**/*.json")).toBe("src/🌎*/**/*.json");
    expect(fixGlobPattern("src/🚀**/*.ts")).toBe("src/🚀*/**/*.ts");
  });

  it("errors if both turbo.json and turbo.jsonc exist", () => {
    const { root, write } = useFixture({ fixture: "clean-globs" });
    write("turbo.jsonc", '{ "pipeline": {} }');

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false }
    });

    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toContain(
      "Found both turbo.json and turbo.jsonc"
    );
  });
});
