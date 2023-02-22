import { RULES } from "../constants";
import getEnvVarDependencies from "../utils/getEnvVarDependencies";

// Add the environment variables into the ESLint incremental cache key.
const envVars = getEnvVarDependencies({
  cwd: process.cwd(),
});
const settings = {
  turbo: {
    envVars: envVars
      ? Object.values(envVars)
          .flatMap((s) => Array.from(s))
          .sort()
      : [],
  },
};

const config = {
  settings,
  plugins: ["turbo"],
  rules: {
    [`turbo/${RULES.noUndeclaredEnvVars}`]: "error",
  },
};

export default config;
