import path from "path";
import fs from "fs-extra";
import { getTurboConfigs } from "@turbo/utils";
import type { Schema as TurboJsonSchema } from "@turbo/types";

import type { TransformerArgs } from "../types";
import getTransformerHelpers from "../utils/getTransformerHelpers";
import { TransformerResults } from "../runner";
import { RootSchema } from "@turbo/types/src/types/config";

// transformer details
const TRANSFORMER = "stabilize-env-mode";
const DESCRIPTION =
  "Rewrite experimentalPassThroughEnv and experimentalGlobalPassThroughEnv";
const INTRODUCED_IN = "1.10.0";

function migrateRootConfig(config: RootSchema) {
  let oldConfig = config.experimentalGlobalPassThroughEnv;
  let newConfig = config.globalPassThroughEnv;
  // Set to an empty array is meaningful, so we have undefined as an option here.
  let output: string[] | undefined;
  if (Array.isArray(oldConfig) || Array.isArray(newConfig)) {
    output = [];

    if (Array.isArray(oldConfig)) {
      output = output.concat(oldConfig);
    }
    if (Array.isArray(newConfig)) {
      output = output.concat(newConfig);
    }

    // Deduplicate
    output = [...new Set(output)];

    output.sort();
  }

  // Can blindly delete and repopulate with calculated value.
  delete config.experimentalGlobalPassThroughEnv;
  delete config.globalPassThroughEnv;

  if (Array.isArray(output)) {
    config.globalPassThroughEnv = output;
  }

  return migrateTaskConfigs(config);
}

function migrateTaskConfigs(config: TurboJsonSchema) {
  for (const [_, taskDef] of Object.entries(config.pipeline)) {
    let oldConfig = taskDef.experimentalPassThroughEnv;
    let newConfig = taskDef.passThroughEnv;

    // Set to an empty array is meaningful, so we have undefined as an option here.
    let output: string[] | undefined;
    if (Array.isArray(oldConfig) || Array.isArray(newConfig)) {
      output = [];

      if (Array.isArray(oldConfig)) {
        output = output.concat(oldConfig);
      }
      if (Array.isArray(newConfig)) {
        output = output.concat(newConfig);
      }

      // Deduplicate
      output = [...new Set(output)];

      // Sort
      output.sort();
    }

    // Can blindly delete and repopulate with calculated value.
    delete taskDef.experimentalPassThroughEnv;
    delete taskDef.passThroughEnv;

    if (Array.isArray(output)) {
      taskDef.passThroughEnv = output;
    }
  }

  return config;
}

export function transformer({
  root,
  options,
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  // If `turbo` key is detected in package.json, require user to run the other codemod first.
  const packageJsonPath = path.join(root, "package.json");
  // package.json should always exist, but if it doesn't, it would be a silly place to blow up this codemod
  let packageJSON = {};

  try {
    packageJSON = fs.readJSONSync(packageJsonPath);
  } catch (e) {
    // readJSONSync probably failed because the file doesn't exist
  }

  if ("turbo" in packageJSON) {
    return runner.abortTransform({
      reason:
        '"turbo" key detected in package.json. Run `npx @turbo/codemod transform create-turbo-config` first',
    });
  }

  log.info(
    "Rewriting `experimentalPassThroughEnv` and `experimentalGlobalPassThroughEnv`"
  );
  const turboConfigPath = path.join(root, "turbo.json");
  if (!fs.existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  const turboJson: RootSchema = fs.readJsonSync(turboConfigPath);
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateRootConfig(turboJson),
  });

  // find and migrate any workspace configs
  const allTurboJsons = getTurboConfigs(root);
  allTurboJsons.forEach((workspaceConfig) => {
    const { config, turboConfigPath, isRootConfig } = workspaceConfig;
    if (!isRootConfig) {
      runner.modifyFile({
        filePath: turboConfigPath,
        after: migrateTaskConfigs(config),
      });
    }
  });

  return runner.finish();
}

const transformerMeta = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
};

export default transformerMeta;
