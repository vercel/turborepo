import path from "node:path";
import { readJsonSync, existsSync } from "fs-extra";
import { type PackageJson, getTurboConfigs } from "@turbo/utils";
import type { Pipeline, SchemaV1 } from "@turbo/types";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { TransformerResults } from "../runner";
import type { Transformer, TransformerArgs } from "../types";
import { loadTurboJson } from "../utils/loadTurboJson";

// transformer details
const TRANSFORMER = "migrate-env-var-dependencies";
const DESCRIPTION =
  'Migrate environment variable dependencies from "dependsOn" to "env" in `turbo.json`';
const INTRODUCED_IN = "1.5.0";

export function hasLegacyEnvVarDependencies(config: SchemaV1) {
  const dependsOn = [
    "extends" in config ? [] : config.globalDependencies,
    Object.values(config.pipeline).flatMap(
      (pipeline) => pipeline.dependsOn ?? []
    ),
  ].flat();
  const envVars = dependsOn.filter((dep) => dep?.startsWith("$"));
  return { hasKeys: Boolean(envVars.length), envVars };
}

export function migrateDependencies({
  env,
  deps,
}: {
  env?: Array<string>;
  deps?: Array<string>;
}) {
  const envDeps = new Set<string>(env);
  const otherDeps: Array<string> = [];
  deps?.forEach((dep) => {
    if (dep.startsWith("$")) {
      envDeps.add(dep.slice(1));
    } else {
      otherDeps.push(dep);
    }
  });
  if (envDeps.size) {
    return {
      deps: otherDeps,
      env: Array.from(envDeps),
    };
  }
  return { env, deps };
}

export function migratePipeline(pipeline: Pipeline) {
  const { deps: dependsOn, env } = migrateDependencies({
    env: pipeline.env,
    deps: pipeline.dependsOn,
  });
  const migratedPipeline = { ...pipeline };
  if (dependsOn) {
    migratedPipeline.dependsOn = dependsOn;
  } else {
    delete migratedPipeline.dependsOn;
  }
  if (env?.length) {
    migratedPipeline.env = env;
  } else {
    delete migratedPipeline.env;
  }

  return migratedPipeline;
}

export function migrateGlobal(config: SchemaV1) {
  if ("extends" in config) {
    return config;
  }

  const { deps: globalDependencies, env } = migrateDependencies({
    env: config.globalEnv,
    deps: config.globalDependencies,
  });
  const migratedConfig = { ...config };
  if (globalDependencies?.length) {
    migratedConfig.globalDependencies = globalDependencies;
  } else {
    delete migratedConfig.globalDependencies;
  }
  if (env?.length) {
    migratedConfig.globalEnv = env;
  } else {
    delete migratedConfig.globalEnv;
  }
  return migratedConfig;
}

export function migrateConfig(config: SchemaV1) {
  const migratedConfig = migrateGlobal(config);
  Object.keys(config.pipeline).forEach((pipelineKey) => {
    config.pipeline;
    if (pipelineKey in config.pipeline) {
      const pipeline = migratedConfig.pipeline[pipelineKey];
      migratedConfig.pipeline[pipelineKey] = {
        ...pipeline,
        ...migratePipeline(pipeline),
      };
    }
  });
  return migratedConfig;
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

  log.info(
    `Migrating environment variable dependencies from "globalDependencies" and "dependsOn" to "env" in "turbo.json"...`
  );

  // validate we don't have a package.json config
  const packageJsonPath = path.join(root, "package.json");
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

  // validate we have a root config
  const turboConfigPath = path.join(root, "turbo.json");
  if (!existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  let turboJson: SchemaV1 = loadTurboJson(turboConfigPath);
  if (hasLegacyEnvVarDependencies(turboJson).hasKeys) {
    turboJson = migrateConfig(turboJson);
  }

  runner.modifyFile({
    filePath: turboConfigPath,
    after: turboJson,
  });

  // find and migrate any workspace configs
  const workspaceConfigs = getTurboConfigs(root);
  workspaceConfigs.forEach((workspaceConfig) => {
    const { config, turboConfigPath: filePath } = workspaceConfig;
    if ("pipeline" in config && hasLegacyEnvVarDependencies(config).hasKeys) {
      runner.modifyFile({
        filePath,
        after: migrateConfig(config),
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
