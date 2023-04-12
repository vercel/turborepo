import fs from "fs-extra";
import path from "path";
import { getTurboConfigs } from "@turbo/utils";
import type { Schema, Pipeline } from "@turbo/types";

import getTransformerHelpers from "../utils/getTransformerHelpers";
import { TransformerResults } from "../runner";
import type { TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "migrate-env-var-dependencies";
const DESCRIPTION =
  'Migrate environment variable dependencies from "dependsOn" to "env" in `turbo.json`';
const INTRODUCED_IN = "1.5.0";

export function hasLegacyEnvVarDependencies(config: Schema) {
  const dependsOn = [
    "extends" in config ? [] : config.globalDependencies,
    Object.values(config.pipeline).flatMap(
      (pipeline) => pipeline.dependsOn ?? []
    ),
  ].flat();
  const envVars = dependsOn.filter((dep) => dep?.startsWith("$"));
  return { hasKeys: !!envVars.length, envVars };
}

export function migrateDependencies({
  env,
  deps,
}: {
  env?: string[];
  deps?: string[];
}) {
  const envDeps: Set<string> = new Set(env);
  const otherDeps: string[] = [];
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
  } else {
    return { env, deps };
  }
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
  if (env && env.length) {
    migratedPipeline.env = env;
  } else {
    delete migratedPipeline.env;
  }

  return migratedPipeline;
}

export function migrateGlobal(config: Schema) {
  if ("extends" in config) {
    return config;
  }

  const { deps: globalDependencies, env } = migrateDependencies({
    env: config.globalEnv,
    deps: config.globalDependencies,
  });
  const migratedConfig = { ...config };
  if (globalDependencies && globalDependencies.length) {
    migratedConfig.globalDependencies = globalDependencies;
  } else {
    delete migratedConfig.globalDependencies;
  }
  if (env && env.length) {
    migratedConfig.globalEnv = env;
  } else {
    delete migratedConfig.globalEnv;
  }
  return migratedConfig;
}

export function migrateConfig(config: Schema) {
  let migratedConfig = migrateGlobal(config);
  Object.keys(config.pipeline).forEach((pipelineKey) => {
    config.pipeline;
    if (migratedConfig.pipeline && config.pipeline[pipelineKey]) {
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

  // validate we have a root config
  const turboConfigPath = path.join(root, "turbo.json");
  if (!fs.existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  let turboJson: Schema = fs.readJsonSync(turboConfigPath);
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
    const { config, turboConfigPath } = workspaceConfig;
    if (hasLegacyEnvVarDependencies(config).hasKeys) {
      runner.modifyFile({
        filePath: turboConfigPath,
        after: migrateConfig(config),
      });
    }
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
