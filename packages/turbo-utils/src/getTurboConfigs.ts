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

const ROOT_GLOB = "turbo.json";
const ROOT_WORKSPACE_GLOB = "package.json";

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
        // eslint-disable-next-line import/no-named-as-default-member -- json5 exports different objects depending on if you're using esm or cjs (https://github.com/json5/json5/issues/240)
        const turboJsonContent: SchemaV1 = JSON5.parse(raw);
        // basic config validation
        const isRootConfig = path.dirname(configPath) === turboRoot;
        if (isRootConfig) {
          // invalid - root config with extends
          if ("extends" in turboJsonContent) {
            return;
          }
        } else if (!("extends" in turboJsonContent)) {
          // invalid - workspace config with no extends
          return;
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

    configPaths.forEach((configPath) => {
      try {
        const rawPackageJson = fs.readFileSync(configPath, "utf8");
        const packageJsonContent = JSON.parse(rawPackageJson) as PackageJson;

        const workspaceName = packageJsonContent.name;
        const workspacePath = path.dirname(configPath);
        const isWorkspaceRoot = workspacePath === turboRoot;

        // Try and get turbo.json
        const turboJsonPath = path.join(workspacePath, "turbo.json");
        let rawTurboJson = null;
        let turboConfig: SchemaV1 | undefined;
        try {
          rawTurboJson = fs.readFileSync(turboJsonPath, "utf8");
          // eslint-disable-next-line import/no-named-as-default-member -- json5 exports different objects depending on if you're using esm or cjs (https://github.com/json5/json5/issues/240)
          turboConfig = JSON5.parse(rawTurboJson);

          if (turboConfig) {
            // basic config validation
            if (isWorkspaceRoot) {
              // invalid - root config with extends
              if ("extends" in turboConfig) {
                return;
              }
            } else if (!("extends" in turboConfig)) {
              // invalid - workspace config with no extends
              return;
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
        logger.warn(e);
      }
    });
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
