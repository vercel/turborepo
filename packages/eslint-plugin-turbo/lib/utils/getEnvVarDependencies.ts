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
  const { globalDependencies, pipeline = {} } = turboJsonContent;

  const allEnvVars: Array<string> = findDependsOnEnvVars({
    dependencies: globalDependencies,
  });
  Object.values(pipeline).forEach(({ dependsOn }) => {
    if (dependsOn) {
      allEnvVars.push(...findDependsOnEnvVars({ dependencies: dependsOn }));
    }
  });

  // remove leading $
  const envVarSet = new Set(
    allEnvVars.map((envVar) => envVar.slice(1, envVar.length))
  );

  return envVarSet;
}

export default getEnvVarDependencies;
