import { getTurboConfigs } from "turbo-utils";
import { Schema } from "turbo-types";

function findDependsOnEnvVars({
  dependencies,
}: {
  dependencies?: Array<string>;
}) {
  if (dependencies) {
    return (
      dependencies
        // filter for dep env vars
        .filter((dep) => dep.startsWith("$"))
        // remove leading $
        .map((envVar) => envVar.slice(1, envVar.length))
    );
  }

  return [];
}

function getEnvVarDependencies({
  cwd,
  turboConfigs,
}: {
  cwd: string | undefined;
  turboConfigs?: Array<Schema>;
}): Set<string> | null {
  let allTurboConfigs = turboConfigs;
  if (!allTurboConfigs) {
    allTurboConfigs = Object.values(getTurboConfigs(cwd));
  }

  if (!allTurboConfigs.length) {
    return new Set();
  }

  const allEnvVars: Array<string> = [];
  allTurboConfigs.forEach((config) => {
    // handle globals
    if (!("extends" in config)) {
      const { globalDependencies = [], globalEnv = [] } = config;

      allEnvVars.push(
        ...findDependsOnEnvVars({
          dependencies: globalDependencies,
        }),
        ...globalEnv
      );
    }

    // handle pipelines
    const { pipeline = {} } = config;
    Object.values(pipeline).forEach(({ env, dependsOn }) => {
      if (dependsOn) {
        allEnvVars.push(...findDependsOnEnvVars({ dependencies: dependsOn }));
      }

      if (env) {
        allEnvVars.push(...env);
      }
    });
  });

  return new Set(allEnvVars);
}

export default getEnvVarDependencies;
