import fs from "fs-extra";
import path from "path";
import { Flags } from "../types";
import type { Schema, Pipeline } from "turbo-types";
import chalk from "chalk";
import { skip, ok, error } from "../logger";

export function hasLegacyEnvVarDependencies(config: Schema) {
  const dependsOn = [
    config.globalDependencies,
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
    if (dep?.startsWith("$")) {
      envDeps.add(dep);
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
  const { deps: globalDependencies, env } = migrateDependencies({
    env: config.env,
    deps: config.globalDependencies,
  });
  const migratedConfig = { ...config };
  if (globalDependencies) {
    migratedConfig.globalDependencies = globalDependencies;
  } else {
    delete migratedConfig.globalDependencies;
  }
  if (env && env.length) {
    migratedConfig.env = env;
  } else {
    delete migratedConfig.env;
  }

  return migratedConfig;
}

export function migrateConfig(config: Schema) {
  let migratedConfig = migrateGlobal(config);
  Object.keys(config.pipeline).forEach((pipelineKey) => {
    if (migratedConfig.pipeline && config.pipeline?.[pipelineKey]) {
      const pipeline = migratedConfig.pipeline?.[pipelineKey];
      migratedConfig.pipeline[pipelineKey] = {
        ...pipeline,
        ...migratePipeline(pipeline),
      };
    }
  });
  return migratedConfig;
}

export default function migrateEnvVarDependencies(
  files: string[],
  flags: Flags
) {
  if (files.length === 1) {
    const dir = files[0];
    const root = path.resolve(process.cwd(), dir);
    console.log(
      `Migrating environment variable dependencies from "globalDependencies" and "dependsOn" to "env" in "turbo.json"...`
    );
    const turboConfigPath = path.join(root, "turbo.json");
    let modifiedCount = 0;
    let skippedCount = 0;
    let unmodifiedCount = 1;
    if (!fs.existsSync(turboConfigPath)) {
      error(`No turbo.json found at ${root}. Is the path correct?`);
      process.exit(1);
    }

    const rootTurboJson = fs.readJsonSync(turboConfigPath);
    if (hasLegacyEnvVarDependencies(rootTurboJson).hasKeys) {
      if (flags.dry) {
        if (flags.print) {
          console.log(JSON.stringify(migrateConfig(rootTurboJson), null, 2));
        }
        skip("turbo.json", chalk.dim("(dry run)"));
        skippedCount += 1;
      } else {
        if (flags.print) {
          console.log(JSON.stringify(migrateConfig(rootTurboJson), null, 2));
        }
        ok("turbo.json");
        fs.writeJsonSync(turboConfigPath, migrateConfig(rootTurboJson), {
          spaces: 2,
        });
        modifiedCount += 1;
        unmodifiedCount -= 1;
      }
    } else {
      ok(
        'no unmigrated environment variable dependencies found in "turbo.json"'
      );
      process.exit(0);
    }

    console.log("All done.");
    console.log("Results:");
    console.log(chalk.red(`0 errors`));
    console.log(chalk.yellow(`${skippedCount} skipped`));
    console.log(chalk.yellow(`${unmodifiedCount} unmodified`));
    console.log(chalk.green(`${modifiedCount} modified`));
  }
}
