import fs from "fs";
import path from "path";
import getTurboRoot from "./getTurboRoot";
import yaml from "js-yaml";
import { sync } from "fast-glob";
import { Schema } from "@turbo/types";
import JSON5 from "json5";

const ROOT_GLOB = "turbo.json";

export type TurboConfigs = Array<{
  config: Schema;
  turboConfigPath: string;
  workspacePath: string;
  isRootConfig: boolean;
}>;

interface Options {
  cache?: boolean;
}

const configsCache: Record<string, TurboConfigs> = {};

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
      );
      return packageJson?.workspaces || [];
    }
  } catch (e) {
    return [];
  }
}

function getTurboConfigs(cwd?: string, opts?: Options): TurboConfigs {
  const turboRoot = getTurboRoot(cwd, opts);
  const configs: TurboConfigs = [];

  const cacheEnabled = opts?.cache ?? true;
  if (cacheEnabled && cwd && configsCache[cwd]) {
    return configsCache[cwd];
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
    configsCache[cwd] = configs;
  }

  return configs;
}

export default getTurboConfigs;
