import findTurboConfig from "./findTurboConfig";
import type { Schema } from "turbo-types";

function findDependsOnEnvVars({
  dependencies,
}: {
  dependencies?: Array<string>;
}) {
  if (dependencies) {
    return dependencies.filter((dep) => dep.startsWith("$"));
  }

  return [];
}

function getEnvVarDependencies({
  cwd,
  turboConfig,
}: {
  cwd: string;
  turboConfig?: Schema;
}): Set<string> | null {
  const turboJsonContent = turboConfig || findTurboConfig({ cwd });
  if (!turboJsonContent) {
    return null;
  }
  const {
    globalDependencies,
    globalEnv = [],
    pipeline = {},
  } = turboJsonContent;

  const allEnvVars: Array<string> = [
    ...findDependsOnEnvVars({
      dependencies: globalDependencies,
    }),
    ...globalEnv,
  ];
  Object.values(pipeline).forEach(({ env, dependsOn }) => {
    if (dependsOn) {
      allEnvVars.push(...findDependsOnEnvVars({ dependencies: dependsOn }));
    }

    if (env) {
      allEnvVars.push(...env);
    }
  });

  // remove leading $, but only for the vars, that are prefixed (from deprecated `dependsOn`)
  const envVarSet = new Set(
    allEnvVars.map((envVar) =>
      envVar.startsWith("$") ? envVar.slice(1, envVar.length) : envVar
    )
  );

  return envVarSet;
}

export default getEnvVarDependencies;
