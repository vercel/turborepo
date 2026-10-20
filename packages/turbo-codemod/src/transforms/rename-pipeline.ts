import { getTurboConfigs, resolveTurboConfigPath } from "@turbo/utils";
import type { SchemaV2, SchemaV1 } from "@turbo/types";
import type { Transformer, TransformerArgs } from "../types";
import { getTransformerHelpers } from "../utils/get-transformer-helpers";
import type { TransformerResults } from "../runner";
import { loadTurboJson } from "../utils/load-turbo-json";
import { isPipelineKeyMissing } from "../utils/is-pipeline-key-missing";

// transformer details
const TRANSFORMER = "rename-pipeline";
const DESCRIPTION = 'Rename the "pipeline" key to "tasks" in `turbo.json`';
const INTRODUCED_IN = "2.0.0-canary.0";

function migrateConfig(config: SchemaV1): SchemaV2 | SchemaV1 {
  if (isPipelineKeyMissing(config)) {
    return config;
  }

  const { pipeline, ...rest } = config;

  return { ...rest, tasks: pipeline };
}

export function transformer({
  root,
  options
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options
  });

  log.info("Renaming `pipeline` key in turbo.json to `tasks`");
  const { configPath: turboConfigPath, error: resolveError } =
    resolveTurboConfigPath(root);
  if (resolveError) {
    return runner.abortTransform({ reason: resolveError });
  }
  if (!turboConfigPath) {
    return runner.abortTransform({
      reason: `No turbo.json or turbo.jsonc found at ${root}. Is the path correct?`
    });
  }

  const _turboJson: SchemaV1 | SchemaV2 = loadTurboJson(turboConfigPath);
  if ("tasks" in _turboJson) {
    // Don't do anything
    log.info("turbo.json already has a tasks key, exiting");
    return runner.finish();
  }

  const turboJson = _turboJson as SchemaV1;
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateConfig(turboJson)
  });

  // find and migrate any workspace configs
  const workspaceConfigs = getTurboConfigs(root);
  for (const workspaceConfig of workspaceConfigs) {
    const { config, turboConfigPath: filePath } = workspaceConfig;
    if ("pipeline" in config) {
      runner.modifyFile({
        filePath,
        after: migrateConfig(config)
      });
    }
  }

  return runner.finish();
}

const transformerMeta: Transformer = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
