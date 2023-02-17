import {
  getTurboRoot,
  getTurboConfigs,
  readTurboConfigPaths,
  TurboConfigs,
} from "turbo-utils";

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
  turboConfigPaths,
  cwd,
}: {
  turboConfigPaths?: Array<string>;
  cwd: string | undefined;
}): Record<string, Set<string>> | null {
  let turboConfigs: TurboConfigs = [];
  if (turboConfigPaths) {
    turboConfigs = readTurboConfigPaths({
      configPaths: turboConfigPaths,
    });
  } else {
    turboConfigs = getTurboConfigs(cwd);
  }

  if (!turboConfigs.length) {
    return null;
  }

  const envVars: Record<string, Set<string>> = {
    "//": new Set(),
  };

  turboConfigs.forEach((turboConfig) => {
    const { config, workspacePath, isRootConfig } = turboConfig;

    const key = isRootConfig ? "//" : workspacePath;
    if (!envVars[key]) {
      envVars[key] = new Set();
    }

    // handle globals
    if (!("extends" in config)) {
      const { globalDependencies = [], globalEnv = [] } = config;

      const keys = [
        ...findDependsOnEnvVars({
          dependencies: globalDependencies,
        }),
        ...globalEnv,
      ];
      keys.forEach((k) => envVars[key].add(k));
    }

    // handle pipelines
    const { pipeline = {} } = config;
    Object.values(pipeline).forEach(({ env, dependsOn }) => {
      if (dependsOn) {
        findDependsOnEnvVars({ dependencies: dependsOn }).forEach((k) =>
          envVars[key].add(k)
        );
      }

      if (env) {
        env.forEach((k) => envVars[key].add(k));
      }
    });
  });

  return envVars;
}

export default getEnvVarDependencies;
