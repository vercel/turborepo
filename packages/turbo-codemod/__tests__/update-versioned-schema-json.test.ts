import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import { transformer } from "../src/transforms/update-versioned-schema-json";

describe("update-versioned-schema-json", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "update-versioned-schema-json"
  });

  it("updates schema URL to versioned subdomain format when toVersion >= 2.7.5", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
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

  it("handles higher versions correctly", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const result = transformer({
      root,
      options: {
        force: false,
        dryRun: false,
        print: false,
        toVersion: "2.10.3"
      }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-10-3.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("strips prerelease suffix from version", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const result = transformer({
      root,
      options: {
        force: false,
        dryRun: false,
        print: false,
        toVersion: "2.7.5-canary.13"
      }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("handles prerelease versions with build metadata", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const result = transformer({
      root,
      options: {
        force: false,
        dryRun: false,
        print: false,
        toVersion: "2.8.0-beta.1+build.123"
      }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-8-0.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("does nothing when toVersion is not specified", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });

  it("does nothing when toVersion is below 2.7.5", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.4" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });

  it("does nothing if schema URL is already the target version", () => {
    const { root, read } = useFixture({
      fixture: "already-versioned"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });

  it("upgrades outdated versioned schema URL to target version", () => {
    const { root, read } = useFixture({
      fixture: "outdated-versioned"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.8.0" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-8-0.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("does not modify v1 schema URLs", () => {
    const { root, read } = useFixture({
      fixture: "v1-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://turborepo.dev/schema.v1.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });

  it("migrates v2 schema URL to versioned format", () => {
    const { root, read } = useFixture({
      fixture: "v2-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("migrates turborepo.com schema URL to versioned format", () => {
    const { root, read } = useFixture({
      fixture: "dotcom-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("migrates turbo.build schema URL to versioned format", () => {
    const { root, read } = useFixture({
      fixture: "turbo-build-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("does nothing if no schema is present", () => {
    const { root, read } = useFixture({
      fixture: "no-schema"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toStrictEqual({});
  });

  it("updates schema URL in workspace turbo.json files", () => {
    const { root, read } = useFixture({
      fixture: "workspace-configs"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    // Root turbo.json should be updated
    expect(JSON.parse(read("turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      tasks: {
        build: {
          outputs: ["dist/**"]
        }
      }
    });

    // Workspace turbo.json files should also be updated
    expect(JSON.parse(read("apps/web/turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      extends: ["//"],
      tasks: {
        build: {
          outputs: [".next/**"]
        }
      }
    });

    expect(JSON.parse(read("apps/docs/turbo.json") || "{}")).toStrictEqual({
      $schema: "https://v2-7-5.turborepo.dev/schema.json",
      extends: ["//"],
      tasks: {
        dev: {
          persistent: true
        }
      }
    });

    expect(result.fatalError).toBeUndefined();
    // Should have 3 modified files
    expect(Object.keys(result.changes).length).toBe(3);
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("reports changes but does not modify files in dryRun mode", () => {
    const { root, read } = useFixture({
      fixture: "old-schema"
    });

    const originalContent = read("turbo.json");

    const result = transformer({
      root,
      options: { force: false, dryRun: true, print: false, toVersion: "2.7.5" }
    });

    // File should be unchanged
    expect(read("turbo.json")).toBe(originalContent);
    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("skipped");
  });

  it("replaces all occurrences of old schema URL in file", () => {
    const { root, read } = useFixture({
      fixture: "multiple-schema-urls"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    const content = read("turbo.json") as string;

    // Both occurrences should be replaced
    expect(content).not.toContain("https://turborepo.dev/schema.json");
    expect(content.match(/v2-7-5\.turborepo\.dev\/schema\.json/g)?.length).toBe(
      2
    );

    expect(result.fatalError).toBeUndefined();
    expect(result.changes["turbo.json"].action).toBe("modified");
  });

  it("aborts with error when getTurboConfigs throws (e.g., both turbo.json and turbo.jsonc exist)", () => {
    const { root } = useFixture({
      fixture: "conflicting-configs"
    });

    const result = transformer({
      root,
      options: { force: false, dryRun: false, print: false, toVersion: "2.7.5" }
    });

    expect(result.fatalError).toBeDefined();
    expect(result.fatalError?.message).toContain("Error updating schema URL");
  });
});
