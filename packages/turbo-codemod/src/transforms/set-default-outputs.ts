import path from "path";
import fs from "fs-extra";
import { getTurboConfigs } from "@turbo/utils";
import type { Schema as TurboJsonSchema } from "@turbo/types";

import type { TransformerArgs } from "../types";
import getTransformerHelpers from "../utils/getTransformerHelpers";
import { TransformerResults } from "../runner";

const DEFAULT_OUTPUTS = ["dist/**", "build/**"];

// transformer details
const TRANSFORMER = "set-default-outputs";
const DESCRIPTION =
  'Add the "outputs" key with defaults where it is missing in `turbo.json`';
const INTRODUCED_IN = "1.7.0";

function migrateConfig(config: TurboJsonSchema) {
  for (const [_, taskDef] of Object.entries(config.pipeline)) {
    if (taskDef.cache !== false) {
      if (!taskDef.outputs) {
        taskDef.outputs = DEFAULT_OUTPUTS;
      } else if (
        Array.isArray(taskDef.outputs) &&
        taskDef.outputs.length === 0
      ) {
        delete taskDef.outputs;
      }
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

  log.info(`Adding default \`outputs\` key into tasks if it doesn't exist`);
  const turboConfigPath = path.join(root, "turbo.json");
  if (!fs.existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  const turboJson: TurboJsonSchema = fs.readJsonSync(turboConfigPath);
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateConfig(turboJson),
  });

  // find and migrate any workspace configs
  const workspaceConfigs = getTurboConfigs(root);
  workspaceConfigs.forEach((workspaceConfig) => {
    const { config, turboConfigPath } = workspaceConfig;
    runner.modifyFile({
      filePath: turboConfigPath,
      after: migrateConfig(config),
    });
  });

  return runner.finish();
}

const transformerMeta = {
  name: `${TRANSFORMER}: ${DESCRIPTION}`,
  value: TRANSFORMER,
  introducedIn: INTRODUCED_IN,
  transformer,
};

export default transformerMeta;
