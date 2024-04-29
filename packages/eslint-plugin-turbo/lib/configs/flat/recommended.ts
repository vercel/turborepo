import { RULES } from "../../constants";
import { Project } from "../../utils/calculate-inputs";
import noUndeclaredEnvVars from "../../rules/no-undeclared-env-vars";

const project = new Project(process.cwd());
const cacheKey = project.valid() ? project.key() : Math.random();

const config = {
  plugins: {
    turbo: {
      // prevent circular dependency when importing from "../.."
      rules: {
        [RULES.noUndeclaredEnvVars]: noUndeclaredEnvVars,
      },
    },
  },
  rules: {
    [`turbo/${RULES.noUndeclaredEnvVars}`]: "error",
  },
  settings: {
    turbo: {
      cacheKey,
    },
  },
};

export default config;
