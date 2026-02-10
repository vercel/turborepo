import { describe, it, mock } from "node:test";
import assert from "node:assert/strict";

interface TestCase {
  command: string;
  options: Record<string, unknown>;
  target: "run" | "workspace";
  calledWith: Record<string, unknown>;
}

const testMatrix: Array<TestCase> = [
  {
    command: "run",
    options: { config: "../config.ts", root: "../" },
    target: "run",
    calledWith: { config: "../config.ts", root: "../" }
  },
  {
    command: "run",
    options: {
      generator_name: "thisOne",
      config: "../config.ts",
      root: "../"
    },
    target: "run",
    calledWith: { config: "../config.ts", root: "../" }
  },
  {
    command: "run",
    options: {
      generator_name: "thisOne",
      config: "../config.ts",
      root: "../",
      args: ["cool name", "packages/cool-name"]
    },
    target: "run",
    calledWith: {
      config: "../config.ts",
      root: "../",
      args: ["cool name", "packages/cool-name"]
    }
  },
  {
    command: "workspace",
    options: {},
    target: "workspace",
    calledWith: {
      empty: true,
      copy: false,
      showAllDependencies: false
    }
  },
  {
    command: "workspace",
    options: { copy: "" },
    target: "workspace",
    calledWith: {
      empty: false,
      copy: true,
      showAllDependencies: false
    }
  },
  {
    command: "workspace",
    options: {
      copy: "some-workspace",
      show_all_dependencies: false
    },
    target: "workspace",
    calledWith: {
      copy: "some-workspace",
      empty: false,
      showAllDependencies: false
    }
  },
  {
    command: "workspace",
    options: {
      type: "package",
      name: "cool-name",
      copy: true,
      show_all_dependencies: true
    },
    target: "workspace",
    calledWith: {
      type: "package",
      name: "cool-name",
      copy: true,
      empty: false,
      showAllDependencies: true
    }
  },
  {
    command: "workspace",
    options: {
      type: "package",
      name: "cool-name",
      empty: true,
      copy: "tailwind-css",
      destination: "../../",
      show_all_dependencies: true,
      example_path: "packages/cool-name"
    },
    target: "workspace",
    calledWith: {
      type: "package",
      name: "cool-name",
      empty: false,
      copy: "tailwind-css",
      destination: "../../",
      showAllDependencies: true,
      examplePath: "packages/cool-name"
    }
  },
  {
    command: "workspace",
    options: {
      type: "package",
      name: "cool-name",
      empty: true,
      copy: "tailwind-css",
      destination: "../../",
      "show-all-dependencies": true,
      "example-path": "packages/cool-name"
    },
    target: "workspace",
    calledWith: {
      type: "package",
      name: "cool-name",
      empty: false,
      copy: "tailwind-css",
      destination: "../../",
      showAllDependencies: true,
      examplePath: "packages/cool-name"
    }
  }
];

const mockRun = mock.fn(async () => {});
const mockWorkspace = mock.fn(async () => {});

// @ts-expect-error -- mock.module exists in Node 22+ but @types/node@18 doesn't include it
mock.module("../src/commands/run/index.js", { namedExports: { run: mockRun } });
// @ts-expect-error -- mock.module exists in Node 22+ but @types/node@18 doesn't include it
mock.module("../src/commands/workspace/index.js", {
  namedExports: { workspace: mockWorkspace }
});

const { raw } = await import("../src/commands/raw/index.js");

describe("raw", () => {
  for (const { command, options, target, calledWith } of testMatrix) {
    it(`${command} with ${JSON.stringify(options)} calls ${target}`, async () => {
      mockRun.mock.resetCalls();
      mockWorkspace.mock.resetCalls();

      await raw(command, { json: JSON.stringify(options) });

      if (target === "run") {
        assert.equal(mockRun.mock.callCount(), 1);
        const runArgs = mockRun.mock.calls[0].arguments as unknown[];
        assert.deepEqual(runArgs[0], options.generator_name);
        assert.deepEqual(runArgs[1], calledWith);
        assert.equal(mockWorkspace.mock.callCount(), 0);
      } else {
        assert.equal(mockWorkspace.mock.callCount(), 1);
        const wsArgs = mockWorkspace.mock.calls[0].arguments as unknown[];
        assert.deepEqual(wsArgs[0], calledWith);
        assert.equal(mockRun.mock.callCount(), 0);
      }
    });
  }
});
