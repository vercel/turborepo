import fs from "fs";
import path from "path";
import { getTurboRoot } from "./getTurboRoot";
import yaml from "js-yaml";
import { sync } from "fast-glob";
import { Schema } from "@turbo/types";
import JSON5 from "json5";

const ROOT_GLOB = "turbo.json";
const ROOT_WORKSPACE_GLOB = "package.json";

export type WorkspaceConfig = {
  workspaceName: string;
  workspacePath: string;
  isWorkspaceRoot: boolean;
  turboConfig?: Schema;
};

export type TurboConfig = {
  config: Schema;
  turboConfigPath: string;
  workspacePath: string;
  isRootConfig: boolean;
};

export type TurboConfigs = Array<TurboConfig>;

interface PackageJson {
  turbo?: Schema;
  workspaces?: { packages: Array<string> } | Array<string>;
}

interface Options {
  cache?: boolean;
}

const turboConfigsCache: Record<string, TurboConfigs> = {};
const workspaceConfigCache: Record<string, WorkspaceConfig[]> = {};

// A quick and dirty workspace parser
// TODO: after @turbo/workspace-convert is merged, we can leverage those utils here
function getWorkspaceGlobs(root: string): Array<string> {
  try {
    if (fs.existsSync(path.join(root, "pnpm-workspace.yaml"))) {
      const workspaceConfig = yaml.load(
        fs.readFileSync(path.join(root, "pnpm-workspace.yaml"), "utf8")
      ) as Record<"packages", Array<string>>;

      return workspaceConfig?.packages || [];
    } else {
      const packageJson = JSON.parse(
        fs.readFileSync(path.join(root, "package.json"), "utf8")
      ) as PackageJson;
      if (packageJson?.workspaces) {
        // support nested packages workspace format
        if ("packages" in packageJson?.workspaces) {
          return packageJson.workspaces.packages || [];
        }
        return packageJson?.workspaces || [];
      }
      return [];
    }
  } catch (e) {
    return [];
  }
}

export function getTurboConfigs(cwd?: string, opts?: Options): TurboConfigs {
  const turboRoot = getTurboRoot(cwd, opts);
  const configs: TurboConfigs = [];

  const cacheEnabled = opts?.cache ?? true;
  if (cacheEnabled && cwd && turboConfigsCache[cwd]) {
    return turboConfigsCache[cwd];
  }

  // parse workspaces
  if (turboRoot) {
    const workspaceGlobs = getWorkspaceGlobs(turboRoot);
    const workspaceConfigGlobs = workspaceGlobs.map(
      (glob) => `${glob}/turbo.json`
    );

    const configPaths = sync([ROOT_GLOB, ...workspaceConfigGlobs], {
      cwd: turboRoot,
      onlyFiles: true,
      followSymbolicLinks: false,
      // avoid throwing when encountering permission errors or unreadable paths
      suppressErrors: true,
    }).map((configPath) => path.join(turboRoot, configPath));

    configPaths.forEach((configPath) => {
      try {
        const raw = fs.readFileSync(configPath, "utf8");
        const turboJsonContent: Schema = JSON5.parse(raw);
        // basic config validation
        let isRootConfig = path.dirname(configPath) === turboRoot;
        if (isRootConfig) {
          // invalid - root config with extends
          if ("extends" in turboJsonContent) {
            return;
          }
        } else {
          // invalid - workspace config with no extends
          if (!("extends" in turboJsonContent)) {
            return;
          }
        }
        configs.push({
          config: turboJsonContent,
          turboConfigPath: configPath,
          workspacePath: path.dirname(configPath),
          isRootConfig,
        });
      } catch (e) {
        // if we can't read or parse the config, just ignore it with a warning
        console.warn(e);
      }
    });
  }

  if (cacheEnabled && cwd) {
    turboConfigsCache[cwd] = configs;
  }

  return configs;
}

export function getWorkspaceConfigs(
  cwd?: string,
  opts?: Options
): WorkspaceConfig[] {
  const turboRoot = getTurboRoot(cwd, opts);
  const configs: WorkspaceConfig[] = [];

  const cacheEnabled = opts?.cache ?? true;
  if (cacheEnabled && cwd && workspaceConfigCache[cwd]) {
    return workspaceConfigCache[cwd];
  }

  // parse workspaces
  if (turboRoot) {
    const workspaceGlobs = getWorkspaceGlobs(turboRoot);
    const workspaceConfigGlobs = workspaceGlobs.map(
      (glob) => `${glob}/package.json`
    );

    const configPaths = sync([ROOT_WORKSPACE_GLOB, ...workspaceConfigGlobs], {
      cwd: turboRoot,
      onlyFiles: true,
      followSymbolicLinks: false,
      // avoid throwing when encountering permission errors or unreadable paths
      suppressErrors: true,
    }).map((configPath) => path.join(turboRoot, configPath));

    configPaths.forEach((configPath) => {
      try {
        const rawPackageJson = fs.readFileSync(configPath, "utf8");
        const packageJsonContent = JSON.parse(rawPackageJson);

        const workspaceName = packageJsonContent.name;
        const workspacePath = path.dirname(configPath);
        const isWorkspaceRoot = workspacePath === turboRoot;

        // Try and get turbo.json
        const turboJsonPath = path.join(workspacePath, "turbo.json");
        let rawTurboJson = null;
        let turboConfig: Schema | undefined;
        try {
          rawTurboJson = fs.readFileSync(turboJsonPath, "utf8");
          turboConfig = JSON5.parse(rawTurboJson);

          if (turboConfig) {
            // basic config validation
            if (isWorkspaceRoot) {
              // invalid - root config with extends
              if ("extends" in turboConfig) {
                return;
              }
            } else {
              // invalid - workspace config with no extends
              if (!("extends" in turboConfig)) {
                return;
              }
            }
          }
        } catch (e) {
          // It is fine for there to not be a turbo.json.
        }

        configs.push({
          workspaceName,
          workspacePath,
          isWorkspaceRoot,
          turboConfig,
        });
      } catch (e) {
        // if we can't read or parse the config, just ignore it with a warning
        console.warn(e);
      }
    });
  }

  if (cacheEnabled && cwd) {
    workspaceConfigCache[cwd] = configs;
  }

  return configs;
}
