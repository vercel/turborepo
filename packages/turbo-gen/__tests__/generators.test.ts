import { describe, it, expect, jest, beforeEach } from "@jest/globals";
import type { DependencyGroups, PackageJson } from "@turbo/utils";
import fs from "fs-extra";
import { gatherAddRequirements } from "../src/utils/gather-add-requirements";
import { generate as copyGenerate } from "../src/generators/copy";
import { generate as emptyGenerate } from "../src/generators/empty";

type MockFn = ReturnType<typeof jest.fn>;

jest.mock("fs-extra", () => ({
  __esModule: true,
  default: {
    readJSON: jest.fn(),
    writeJSON: jest.fn(),
    copy: jest.fn(),
    existsSync: jest.fn(),
    mkdirSync: jest.fn(),
    writeFileSync: jest.fn(),
    rm: jest.fn()
  }
}));

jest.mock("@turbo/utils", () => ({
  logger: {
    log: jest.fn(),
    warn: jest.fn(),
    dimmed: jest.fn(),
    error: jest.fn(),
    turboLoader: jest.fn(() => ({ start: jest.fn(), stop: jest.fn() })),
    turboGradient: jest.fn((s: string) => s)
  },
  createProject: jest.fn()
}));

jest.mock("picocolors", () => ({
  __esModule: true,
  default: { bold: jest.fn((s: string) => s) }
}));

jest.mock("../src/utils/gather-add-requirements");

const mockedGather = jest.mocked(gatherAddRequirements);

// fs-extra has overloaded method signatures that jest.mocked can't resolve,
// so we cast the individual methods we need.
const mockedReadJSON = fs.readJSON as unknown as MockFn;
const mockedWriteJSON = fs.writeJSON as unknown as MockFn;
const mockedCopy = fs.copy as unknown as MockFn;
const mockedWriteFileSync = fs.writeFileSync as unknown as MockFn;

function stubProject(): Parameters<typeof copyGenerate>[0]["project"] {
  return {} as Parameters<typeof copyGenerate>[0]["project"];
}

function stubOpts(
  overrides: Partial<Parameters<typeof copyGenerate>[0]["opts"]> = {}
): Parameters<typeof copyGenerate>[0]["opts"] {
  return {
    method: "copy",
    copy: { type: "internal", source: "source-pkg" },
    showAllDependencies: false,
    ...overrides
  } as Parameters<typeof copyGenerate>[0]["opts"];
}

describe("copy generator — dependency merging", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockedCopy.mockResolvedValue(undefined);
    mockedWriteJSON.mockResolvedValue(undefined);
  });

  it("merges user-selected deps with existing source deps", async () => {
    const sourcePackageJson: PackageJson = {
      name: "source-pkg",
      version: "1.0.0",
      dependencies: {
        react: "^18.0.0",
        "shared-lib": "^1.0.0"
      },
      devDependencies: {
        typescript: "^5.0.0"
      }
    };

    const userDeps: DependencyGroups = {
      dependencies: { "@repo/utils": "workspace:*" },
      devDependencies: {},
      peerDependencies: {},
      optionalDependencies: {}
    };

    mockedGather.mockResolvedValue({
      type: "package",
      name: "new-pkg",
      location: { absolute: "/tmp/new-pkg", relative: "packages/new-pkg" },
      source: {
        name: "source-pkg",
        paths: {
          root: "/tmp/source-pkg",
          packageJson: "/tmp/source-pkg/package.json",
          nodeModules: "/tmp/source-pkg/node_modules"
        }
      },
      dependencies: userDeps
    });

    mockedReadJSON.mockResolvedValue(structuredClone(sourcePackageJson));

    await copyGenerate({
      project: stubProject(),
      opts: stubOpts()
    });

    expect(mockedWriteJSON).toHaveBeenCalledTimes(1);
    const written = mockedWriteJSON.mock.calls[0]![1] as PackageJson;

    // Source deps preserved
    expect(written.dependencies?.react).toBe("^18.0.0");
    expect(written.dependencies?.["shared-lib"]).toBe("^1.0.0");
    // User dep added
    expect(written.dependencies?.["@repo/utils"]).toBe("workspace:*");
    // Source devDeps preserved (empty user devDeps shouldn't wipe them)
    expect(written.devDependencies?.typescript).toBe("^5.0.0");
  });

  it("user-selected deps override source deps at the same key", async () => {
    const sourcePackageJson: PackageJson = {
      name: "source-pkg",
      version: "1.0.0",
      dependencies: {
        "shared-lib": "^1.0.0",
        lodash: "^4.0.0"
      }
    };

    const userDeps: DependencyGroups = {
      dependencies: { "shared-lib": "workspace:*" },
      devDependencies: {},
      peerDependencies: {},
      optionalDependencies: {}
    };

    mockedGather.mockResolvedValue({
      type: "package",
      name: "new-pkg",
      location: { absolute: "/tmp/new-pkg", relative: "packages/new-pkg" },
      source: {
        name: "source-pkg",
        paths: {
          root: "/tmp/source-pkg",
          packageJson: "/tmp/source-pkg/package.json",
          nodeModules: "/tmp/source-pkg/node_modules"
        }
      },
      dependencies: userDeps
    });

    mockedReadJSON.mockResolvedValue(structuredClone(sourcePackageJson));

    await copyGenerate({
      project: stubProject(),
      opts: stubOpts()
    });

    const written = mockedWriteJSON.mock.calls[0]![1] as PackageJson;

    // User selection overrides source version
    expect(written.dependencies?.["shared-lib"]).toBe("workspace:*");
    // Non-overlapping source dep preserved
    expect(written.dependencies?.lodash).toBe("^4.0.0");
  });

  it("handles source package with no pre-existing dependency group", async () => {
    const sourcePackageJson: PackageJson = {
      name: "source-pkg",
      version: "1.0.0"
      // no dependencies field at all
    };

    const userDeps: DependencyGroups = {
      dependencies: { "@repo/ui": "workspace:*" },
      devDependencies: {},
      peerDependencies: {},
      optionalDependencies: {}
    };

    mockedGather.mockResolvedValue({
      type: "package",
      name: "new-pkg",
      location: { absolute: "/tmp/new-pkg", relative: "packages/new-pkg" },
      source: {
        name: "source-pkg",
        paths: {
          root: "/tmp/source-pkg",
          packageJson: "/tmp/source-pkg/package.json",
          nodeModules: "/tmp/source-pkg/node_modules"
        }
      },
      dependencies: userDeps
    });

    mockedReadJSON.mockResolvedValue(structuredClone(sourcePackageJson));

    await copyGenerate({
      project: stubProject(),
      opts: stubOpts()
    });

    const written = mockedWriteJSON.mock.calls[0]![1] as PackageJson;
    expect(written.dependencies?.["@repo/ui"]).toBe("workspace:*");
  });
});

describe("empty generator — dependency merging", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("adds user-selected deps to the scaffolded package.json", async () => {
    const userDeps: DependencyGroups = {
      dependencies: { "@repo/utils": "workspace:*" },
      devDependencies: { "@repo/tsconfig": "workspace:*" },
      peerDependencies: {},
      optionalDependencies: {}
    };

    mockedGather.mockResolvedValue({
      type: "package",
      name: "new-pkg",
      location: { absolute: "/tmp/new-pkg", relative: "packages/new-pkg" },
      source: undefined,
      dependencies: userDeps
    });

    await emptyGenerate({
      project: stubProject(),
      opts: stubOpts({ method: "empty" })
    });

    expect(mockedWriteFileSync).toHaveBeenCalled();

    // Find the package.json write (first writeFileSync call is package.json)
    const packageJsonCall = (
      mockedWriteFileSync.mock.calls as [string, string][]
    ).find(([filePath]) => filePath.endsWith("package.json"));

    expect(packageJsonCall).toBeDefined();
    const written = JSON.parse(packageJsonCall![1]) as PackageJson;

    expect(written.name).toBe("new-pkg");
    expect(written.dependencies?.["@repo/utils"]).toBe("workspace:*");
    expect(written.devDependencies?.["@repo/tsconfig"]).toBe("workspace:*");
  });
});
