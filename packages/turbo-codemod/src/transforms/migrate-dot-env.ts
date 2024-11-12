import path from "node:path";
import { readJsonSync, existsSync } from "fs-extra";
import {
  type PackageJson,
  getTurboConfigs,
  forEachTaskDef,
} from "@turbo/utils";
import { type SchemaV1 } from "@turbo/types";
import type { Transformer, TransformerArgs } from "../types";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { TransformerResults } from "../runner";
import { loadTurboJson } from "../utils/loadTurboJson";

// transformer details
const TRANSFORMER = "migrate-dot-env";
const DESCRIPTION = 'Migrate the "dotEnv" entries to "inputs" in `turbo.json`';
const INTRODUCED_IN = "2.0.0-canary.0";

function migrateConfig(configV1: SchemaV1) {
  if ("globalDotEnv" in configV1) {
    if (configV1.globalDotEnv) {
      configV1.globalDependencies = configV1.globalDependencies ?? [];
      for (const dotEnvPath of configV1.globalDotEnv) {
        configV1.globalDependencies.push(dotEnvPath);
      }
    }
    delete configV1.globalDotEnv;
  }

  forEachTaskDef(configV1, ([_, taskDef]) => {
    if ("dotEnv" in taskDef) {
      if (taskDef.dotEnv) {
        taskDef.inputs = taskDef.inputs ?? ["$TURBO_DEFAULT$"];
        for (const dotEnvPath of taskDef.dotEnv) {
          taskDef.inputs.push(dotEnvPath);
        }
      }
      delete taskDef.dotEnv;
    }
  });

  return configV1;
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
    packageJSON = readJsonSync(packageJsonPath) as PackageJson;
  } catch (e) {
    // readJSONSync probably failed because the file doesn't exist
  }

  if ("turbo" in packageJSON) {
    return runner.abortTransform({
      reason:
        '"turbo" key detected in package.json. Run `npx @turbo/codemod transform create-turbo-config` first',
    });
  }

  log.info(`Moving entries in \`dotEnv\` key in task config to \`inputs\``);
  const turboConfigPath = path.join(root, "turbo.json");
  if (!existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  const turboJson = loadTurboJson<SchemaV1>(turboConfigPath);
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateConfig(turboJson),
  });

  // find and migrate any workspace configs
  const workspaceConfigs = getTurboConfigs(root);
  workspaceConfigs.forEach((workspaceConfig) => {
    const { config, turboConfigPath: filePath } = workspaceConfig;
    runner.modifyFile({
      filePath,
      after: migrateConfig(config),
    });
  });

  return runner.finish();
}

const transformerMeta: Transformer = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
