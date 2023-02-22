import fs from "fs";
import path from "path";
import getTurboRoot from "./getTurboRoot";
import yaml from "js-yaml";
import globby from "globby";
import { Schema } from "turbo-types";
import JSON5 from "json5";

const ROOT_GLOB = "turbo.json";

export type TurboConfigs = Array<{
  config: Schema;
  turboConfigPath: string;
  workspacePath: string;
  isRootConfig: boolean;
}>;

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

function getTurboConfigs(cwd?: string): TurboConfigs {
  const turboRoot = getTurboRoot(cwd);
  const configs: TurboConfigs = [];

  // parse workspaces
  if (turboRoot) {
    const workspaceGlobs = getWorkspaceGlobs(turboRoot);
    const workspaceConfigGlobs = workspaceGlobs.map(
      (glob) => `${glob}/turbo.json`
    );

    const configPaths = globby
      .sync([ROOT_GLOB, ...workspaceConfigGlobs], {
        cwd: turboRoot,
        onlyFiles: true,
        followSymbolicLinks: false,
        gitignore: true,
      })
      .map((configPath) => path.join(turboRoot, configPath));

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
          isRootConfig: !("extends" in turboJsonContent),
        });
      } catch (e) {
        // if we can't parse the config, just ignore it
        console.error(e);
      }
    });
  }

  return configs;
}

export default getTurboConfigs;
