import fs from "node:fs";
import path from "node:path";
import yaml from "js-yaml";
import { sync } from "fast-glob";
import JSON5 from "json5";
import type {
  BaseSchemaV1,
  PipelineV1,
  SchemaV1,
  BaseSchemaV2,
  PipelineV2,
} from "@turbo/types";
import * as logger from "./logger";
import { getTurboRoot } from "./getTurboRoot";
import type { PackageJson, PNPMWorkspaceConfig } from "./types";

const ROOT_GLOB = "{turbo.json,turbo.jsonc}";
const ROOT_WORKSPACE_GLOB = "package.json";

/**
 * Given a directory path, determines which turbo config file to use.
 * Returns error information if both turbo.json and turbo.jsonc exist in the same directory.
 * Returns the path to the config file to use, or null if neither exists.
 */
function resolveTurboConfigPath(dirPath: string): {
  configPath: string | null;
  configExists: boolean;
  error?: string;
} {
  const turboJsonPath = path.join(dirPath, "turbo.json");
  const turboJsoncPath = path.join(dirPath, "turbo.jsonc");

  const turboJsonExists = fs.existsSync(turboJsonPath);
  const turboJsoncExists = fs.existsSync(turboJsoncPath);

  if (turboJsonExists && turboJsoncExists) {
    const errorMessage = `Found both turbo.json and turbo.jsonc in the same directory: ${dirPath}\nPlease use either turbo.json or turbo.jsonc, but not both.`;
    return { configPath: null, configExists: false, error: errorMessage };
  }

  if (turboJsonExists) {
    return { configPath: turboJsonPath, configExists: true };
  }

  if (turboJsoncExists) {
    return { configPath: turboJsoncPath, configExists: true };
  }

  return { configPath: null, configExists: false };
}

export interface WorkspaceConfig {
  workspaceName: string;
  workspacePath: string;
  isWorkspaceRoot: boolean;
  turboConfig?: SchemaV1;
}

export interface TurboConfig {
  config: SchemaV1;
  turboConfigPath: string;
  workspacePath: string;
  isRootConfig: boolean;
}

export type TurboConfigs = Array<TurboConfig>;

interface Options {
  cache?: boolean;
}

const turboConfigsCache: Record<string, TurboConfigs> = {};
const workspaceConfigCache: Record<string, Array<WorkspaceConfig>> = {};

// A quick and dirty workspace parser
// TODO: after @turbo/workspace-convert is merged, we can leverage those utils here
function getWorkspaceGlobs(root: string): Array<string> {
  try {
    if (fs.existsSync(path.join(root, "pnpm-workspace.yaml"))) {
      const workspaceConfig = yaml.load(
        fs.readFileSync(path.join(root, "pnpm-workspace.yaml"), "utf8")
      ) as PNPMWorkspaceConfig;

      return workspaceConfig.packages || [];
    }
    const packageJson = JSON.parse(
      fs.readFileSync(path.join(root, "package.json"), "utf8")
    ) as PackageJson;
    if (packageJson.workspaces) {
      // support nested packages workspace format
      if ("packages" in packageJson.workspaces) {
        return packageJson.workspaces.packages || [];
      }

      if (Array.isArray(packageJson.workspaces)) {
        return packageJson.workspaces;
      }
    }
    return [];
  } catch (e) {
    return [];
  }
}

export function getTurboConfigs(cwd?: string, opts?: Options): TurboConfigs {
  const turboRoot = getTurboRoot(cwd, opts);
  const configs: TurboConfigs = [];

  const cacheEnabled = opts?.cache ?? true;
  if (cacheEnabled && cwd && cwd in turboConfigsCache) {
    return turboConfigsCache[cwd];
  }

  // parse workspaces
  if (turboRoot) {
    const workspaceGlobs = getWorkspaceGlobs(turboRoot);
    const workspaceConfigGlobs = workspaceGlobs.map(
      (glob) => `${glob}/${ROOT_GLOB}`
    );

    const configPaths = sync([ROOT_GLOB, ...workspaceConfigGlobs], {
      cwd: turboRoot,
      onlyFiles: true,
      followSymbolicLinks: false,
      // avoid throwing when encountering permission errors or unreadable paths
      suppressErrors: true,
    }).map((configPath) => path.join(turboRoot, configPath));

    // Check for both turbo.json and turbo.jsonc in the same directory
    const configPathsByDir: Record<string, Array<string>> = {};

    // Group config paths by directory
    for (const configPath of configPaths) {
      const dir = path.dirname(configPath);
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- configPathsByDir[dir] can be undefined
      if (!configPathsByDir[dir]) {
        configPathsByDir[dir] = [];
      }
      configPathsByDir[dir].push(configPath);
    }

    // Process each directory
    for (const [dir, dirConfigPaths] of Object.entries(configPathsByDir)) {
      // If both turbo.json and turbo.jsonc exist in the same directory, throw an error
      if (dirConfigPaths.length > 1) {
        const errorMessage = `Found both turbo.json and turbo.jsonc in the same directory: ${dir}\nPlease use either turbo.json or turbo.jsonc, but not both.`;
        logger.error(errorMessage);
        throw new Error(errorMessage);
      }

      const configPath = dirConfigPaths[0];
      try {
        const raw = fs.readFileSync(configPath, "utf8");

        const turboJsonContent: SchemaV1 = JSON5.parse(raw);
        // basic config validation
        const isRootConfig = path.dirname(configPath) === turboRoot;
        if (isRootConfig) {
          // invalid - root config with extends
          if ("extends" in turboJsonContent) {
            continue;
          }
        } else if (!("extends" in turboJsonContent)) {
          // invalid - workspace config with no extends
          continue;
        }
        configs.push({
          config: turboJsonContent,
          turboConfigPath: configPath,
          workspacePath: path.dirname(configPath),
          isRootConfig,
        });
      } catch (e) {
        // if we can't read or parse the config, just ignore it with a warning
        logger.warn(e);
      }
    }
  }

  if (cacheEnabled && cwd) {
    turboConfigsCache[cwd] = configs;
  }

  return configs;
}

export function getWorkspaceConfigs(
  cwd?: string,
  opts?: Options
): Array<WorkspaceConfig> {
  const turboRoot = getTurboRoot(cwd, opts);
  const configs: Array<WorkspaceConfig> = [];

  const cacheEnabled = opts?.cache ?? true;
  if (cacheEnabled && cwd && cwd in workspaceConfigCache) {
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

    for (const configPath of configPaths) {
      try {
        const rawPackageJson = fs.readFileSync(configPath, "utf8");
        const packageJsonContent = JSON.parse(rawPackageJson) as PackageJson;

        const workspaceName = packageJsonContent.name;
        const workspacePath = path.dirname(configPath);
        const isWorkspaceRoot = workspacePath === turboRoot;

        // Try and get turbo.json or turbo.jsonc
        const {
          configPath: turboConfigPath,
          configExists,
          error,
        } = resolveTurboConfigPath(workspacePath);

        let rawTurboJson = null;
        let turboConfig: SchemaV1 | undefined;

        try {
          // TODO: Our code was allowing both config files to exist. This is a bug, needs to be fixed.
          if (error) {
            logger.error(error);
            throw new Error(error);
          }

          if (configExists && turboConfigPath) {
            rawTurboJson = fs.readFileSync(turboConfigPath, "utf8");
          }

          if (rawTurboJson) {
            turboConfig = JSON5.parse(rawTurboJson);

            if (turboConfig) {
              // basic config validation
              if (isWorkspaceRoot) {
                // invalid - root config with extends
                if ("extends" in turboConfig) {
                  continue;
                }
              } else if (!("extends" in turboConfig)) {
                // invalid - workspace config with no extends
                continue;
              }
            }
          }
        } catch (e) {
          // It is fine for there to not be a turbo.json or turbo.jsonc.
        }

        configs.push({
          workspaceName,
          workspacePath,
          isWorkspaceRoot,
          turboConfig,
        });
      } catch (e) {
        // if we can't read or parse the config, just ignore it with a warning
        logger.warn(e);
      }
    }
  }

  if (cacheEnabled && cwd) {
    workspaceConfigCache[cwd] = configs;
  }

  return configs;
}

export function forEachTaskDef<BaseSchema extends BaseSchemaV1 | BaseSchemaV2>(
  config: BaseSchema,
  f: (
    value: [string, BaseSchema extends BaseSchemaV1 ? PipelineV1 : PipelineV2]
  ) => void
): void {
  if ("pipeline" in config) {
    Object.entries(config.pipeline).forEach(f);
  } else {
    Object.entries(config.tasks).forEach(f);
  }
}

export function clearConfigCaches(): void {
  Object.keys(turboConfigsCache).forEach((key) => {
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- This is safe.
    delete turboConfigsCache[key];
  });
  Object.keys(workspaceConfigCache).forEach((key) => {
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- This is safe.
    delete workspaceConfigCache[key];
  });
}
