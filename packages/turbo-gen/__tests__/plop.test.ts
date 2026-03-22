import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "@jest/globals";
import type { Project } from "@turbo/workspaces";
import nodePlop from "node-plop";
import { Separator } from "@inquirer/prompts";
import { getCustomGenerators } from "../src/utils/plop";
import { getWorkspaceDetailsMockReturnValue } from "./test-utils";

type NodePlopMock = {
  __setConfig: (
    configPath: string,
    handler: (api: {
      setGenerator: (name: string, config: unknown) => void;
    }) => void
  ) => void;
  __reset: () => void;
};

const nodePlopMock = nodePlop as unknown as NodePlopMock;

describe("getCustomGenerators", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "turbo-gen-plop-test-"));
  });

  afterEach(() => {
    nodePlopMock.__reset();
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("keeps duplicate generator names and qualifies display names by workspace", async () => {
    const workspaceNames = ["webapp-customer", "ui-kit"] as const;

    const workspaces = workspaceNames.map((workspaceName) => {
      const workspaceRoot = path.join(tmpDir, "packages", workspaceName);
      const generatorConfigPath = path.join(
        workspaceRoot,
        "turbo",
        "generators",
        "config.ts"
      );
      fs.mkdirSync(path.dirname(generatorConfigPath), { recursive: true });
      fs.writeFileSync(generatorConfigPath, "export default function(){}");

      nodePlopMock.__setConfig(generatorConfigPath, (plop) => {
        plop.setGenerator("component", {
          description: `component in ${workspaceName}`
        });
      });

      return {
        name: workspaceName,
        paths: {
          root: workspaceRoot,
          packageJson: path.join(workspaceRoot, "package.json"),
          nodeModules: path.join(workspaceRoot, "node_modules")
        }
      };
    });

    const project = {
      ...getWorkspaceDetailsMockReturnValue({
        root: tmpDir,
        packageManager: "npm"
      }),
      workspaceData: {
        globs: ["packages/*"],
        workspaces
      }
    } as unknown as Project;

    const generators = await getCustomGenerators({ project });
    const generatorEntries = generators.filter(
      (entry): entry is Exclude<(typeof generators)[number], Separator> =>
        !(entry instanceof Separator)
    );

    expect(generatorEntries).toHaveLength(2);
    expect(generatorEntries.map((entry) => entry.displayName).sort()).toEqual([
      "component (ui-kit)",
      "component (webapp-customer)"
    ]);
    expect(new Set(generatorEntries.map((entry) => entry.name)).size).toBe(2);
  });
});
