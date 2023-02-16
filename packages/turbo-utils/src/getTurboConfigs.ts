import fs from "fs";
import path from "path";
import getTurboRoot from "./getTurboRoot";
import yaml from "js-yaml";
import globby from "globby";
import { Schema } from "turbo-types";
import JSON5 from "json5";

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

function getTurboConfigs(cwd?: string): Record<string, Schema> {
  const root = getTurboRoot(cwd);
  const configs: Record<string, Schema> = {};
  // parse workspaces
  if (root) {
    const workspaceGlobs = getWorkspaceGlobs(root);
    const workspaceConfigGlobs = workspaceGlobs.map(
      (glob) => `${glob}/turbo.json`
    );
    const rootGlob = "turbo.json";
    const configPaths = globby.sync([rootGlob, ...workspaceConfigGlobs], {
      cwd: root,
      onlyFiles: true,
      followSymbolicLinks: false,
      gitignore: true,
    });

    configPaths.forEach((configPath) => {
      try {
        const raw = fs.readFileSync(path.join(root, configPath), "utf8");
        const turboJsonContent: Schema = JSON5.parse(raw);
        configs[configPath] = turboJsonContent;
      } catch (e) {
        // if we can't parse the config, just ignore it
        console.error(e);
      }
    });
  }
  return configs;
}

export default getTurboConfigs;
