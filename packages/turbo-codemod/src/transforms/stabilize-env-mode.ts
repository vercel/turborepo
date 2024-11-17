import path from "node:path";
import { readJsonSync, existsSync } from "fs-extra";
import { type PackageJson, getTurboConfigs } from "@turbo/utils";
import type { SchemaV1, RootSchemaV1, Pipeline } from "@turbo/types";
import type { Transformer, TransformerArgs } from "../types";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { TransformerResults } from "../runner";
import { loadTurboJson } from "../utils/loadTurboJson";

// transformer details
const TRANSFORMER = "stabilize-env-mode";
const DESCRIPTION =
  "Rewrite experimentalPassThroughEnv and experimentalGlobalPassThroughEnv";
const INTRODUCED_IN = "1.10.0";

type ExperimentalRootSchema = Omit<RootSchemaV1, "pipeline"> & {
  experimentalGlobalPassThroughEnv?: null | Array<string>;
  pipeline: Record<string, ExperimentalPipeline>;
};

type ExperimentalPipeline = Pipeline & {
  experimentalPassThroughEnv?: null | Array<string>;
};

type ExperimentalSchema = Omit<SchemaV1, "pipeline"> & {
  pipeline: Record<string, ExperimentalPipeline>;
};

function migrateRootConfig(config: ExperimentalRootSchema) {
  const oldConfig = config.experimentalGlobalPassThroughEnv;
  const newConfig = config.globalPassThroughEnv;
  // Set to an empty array is meaningful, so we have undefined as an option here.
  let output: Array<string> | undefined;
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

function migrateTaskConfigs(config: ExperimentalSchema) {
  for (const [_, taskDef] of Object.entries(config.pipeline)) {
    const oldConfig = taskDef.experimentalPassThroughEnv;
    const newConfig = taskDef.passThroughEnv;

    // Set to an empty array is meaningful, so we have undefined as an option here.
    let output: Array<string> | undefined;
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

  log.info(
    "Rewriting `experimentalPassThroughEnv` and `experimentalGlobalPassThroughEnv`"
  );
  const turboConfigPath = path.join(root, "turbo.json");
  if (!existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  const turboJson: SchemaV1 = loadTurboJson(turboConfigPath);
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateRootConfig(turboJson),
  });

  // find and migrate any workspace configs
  const allTurboJsons = getTurboConfigs(root);
  allTurboJsons.forEach((workspaceConfig) => {
    const { config, turboConfigPath: filePath, isRootConfig } = workspaceConfig;
    if (!isRootConfig && "pipeline" in config) {
      runner.modifyFile({
        filePath,
        after: migrateTaskConfigs(config),
      });
    }
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
