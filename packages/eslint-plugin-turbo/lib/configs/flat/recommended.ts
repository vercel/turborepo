import type { Linter } from "eslint";
import { RULES } from "../../constants";
import { Project } from "../../utils/calculate-inputs";

const project = new Project(process.cwd());
const cacheKey = project.valid() ? project.key() : Math.random();

const config = {
  name: "turbo/recommended",
  rules: {
    [`turbo/${RULES.noUndeclaredEnvVars}`]: "error",
  },
  settings: {
    turbo: {
      cacheKey,
    },
  },
} satisfies Linter.FlatConfig;

export default config;
