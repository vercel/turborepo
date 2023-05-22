import { EnvWildcard } from "@turbo/types/src/types/config";
import { wildcardTests } from "./wildcard-processing";

const globalDefaultEnv: EnvWildcard[] = ["VERCEL_ANALYTICS_ID"];
export function isGlobalAccountedFor(
  globalEnv: EnvWildcard[],
  globalPassThroughEnv: EnvWildcard[]
): (varName: string) => boolean {
  let globalEnvTests = wildcardTests(globalEnv);
  let globalPassThroughEnvTests = wildcardTests(globalPassThroughEnv);
  let globalDefaultEnvTests = wildcardTests(globalDefaultEnv);

  return (varName: string) =>
    globalEnvTests.inclusions.test(varName) ||
    globalEnvTests.exclusions.test(varName) ||
    globalPassThroughEnvTests.inclusions.test(varName) ||
    globalPassThroughEnvTests.exclusions.test(varName) ||
    globalDefaultEnvTests.inclusions.test(varName) ||
    globalDefaultEnvTests.exclusions.test(varName);
}

export function isTaskAccountedFor(
  env: EnvWildcard[],
  passThroughEnv: EnvWildcard[]
): (varName: string) => boolean {
  let envTests = wildcardTests(env);
  let passThroughEnvTests = wildcardTests(passThroughEnv);

  // TODO: does not handle framework inference.

  return (varName: string) =>
    envTests.inclusions.test(varName) ||
    envTests.exclusions.test(varName) ||
    passThroughEnvTests.inclusions.test(varName) ||
    passThroughEnvTests.exclusions.test(varName);
}
