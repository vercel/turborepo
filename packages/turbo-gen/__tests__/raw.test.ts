import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { parseWorkspaceArgs, parseRunArgs } from "../src/commands/raw/index.js";

describe("parseRunArgs", () => {
  it("parses basic run options", () => {
    const result = parseRunArgs(
      JSON.stringify({ config: "../config.ts", root: "../" })
    );
    assert.equal(result.generatorName, undefined);
    assert.deepEqual(result.rest, { config: "../config.ts", root: "../" });
  });

  it("extracts generator_name and camelCases it", () => {
    const result = parseRunArgs(
      JSON.stringify({
        generator_name: "thisOne",
        config: "../config.ts",
        root: "../"
      })
    );
    assert.equal(result.generatorName, "thisOne");
    assert.deepEqual(result.rest, { config: "../config.ts", root: "../" });
  });

  it("passes through args array", () => {
    const result = parseRunArgs(
      JSON.stringify({
        generator_name: "thisOne",
        config: "../config.ts",
        root: "../",
        args: ["cool name", "packages/cool-name"]
      })
    );
    assert.equal(result.generatorName, "thisOne");
    assert.deepEqual(result.rest, {
      config: "../config.ts",
      root: "../",
      args: ["cool name", "packages/cool-name"]
    });
  });
});

describe("parseWorkspaceArgs", () => {
  it("defaults to empty workspace", () => {
    const result = parseWorkspaceArgs(JSON.stringify({}));
    assert.deepEqual(result, {
      empty: true,
      copy: false,
      showAllDependencies: false
    });
  });

  it("empty string copy sets copy=true, empty=false", () => {
    const result = parseWorkspaceArgs(JSON.stringify({ copy: "" }));
    assert.deepEqual(result, {
      empty: false,
      copy: true,
      showAllDependencies: false
    });
  });

  it("string copy value passes through", () => {
    const result = parseWorkspaceArgs(
      JSON.stringify({ copy: "some-workspace", show_all_dependencies: false })
    );
    assert.deepEqual(result, {
      copy: "some-workspace",
      empty: false,
      showAllDependencies: false
    });
  });

  it("boolean copy=true with other options", () => {
    const result = parseWorkspaceArgs(
      JSON.stringify({
        type: "package",
        name: "cool-name",
        copy: true,
        show_all_dependencies: true
      })
    );
    assert.deepEqual(result, {
      type: "package",
      name: "cool-name",
      copy: true,
      empty: false,
      showAllDependencies: true
    });
  });

  it("copy overrides empty when both provided", () => {
    const result = parseWorkspaceArgs(
      JSON.stringify({
        type: "package",
        name: "cool-name",
        empty: true,
        copy: "tailwind-css",
        destination: "../../",
        show_all_dependencies: true,
        example_path: "packages/cool-name"
      })
    );
    assert.deepEqual(result, {
      type: "package",
      name: "cool-name",
      empty: false,
      copy: "tailwind-css",
      destination: "../../",
      showAllDependencies: true,
      examplePath: "packages/cool-name"
    });
  });

  it("handles kebab-case keys", () => {
    const result = parseWorkspaceArgs(
      JSON.stringify({
        type: "package",
        name: "cool-name",
        empty: true,
        copy: "tailwind-css",
        destination: "../../",
        "show-all-dependencies": true,
        "example-path": "packages/cool-name"
      })
    );
    assert.deepEqual(result, {
      type: "package",
      name: "cool-name",
      empty: false,
      copy: "tailwind-css",
      destination: "../../",
      showAllDependencies: true,
      examplePath: "packages/cool-name"
    });
  });
});
