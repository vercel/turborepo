import type { ESLint } from "eslint";
import { version } from "../package.json";
import { RULES } from "./constants";
import noUndeclaredEnvVars from "./rules/no-undeclared-env-vars";
import recommended from "./configs/recommended";
import flatRecommended from "./configs/flat/recommended";

export const rules = {
  [RULES.noUndeclaredEnvVars]: noUndeclaredEnvVars,
};

const plugin = {
  meta: {
    name: "turbo",
    version,
  },
  rules,
  configs: {
    recommended,
    "flat/recommended": {
      ...flatRecommended,
      plugins: {
        get turbo(): ESLint.Plugin {
          return plugin;
        },
      },
    },
  },
} satisfies ESLint.Plugin;

export const { configs } = plugin;

export default plugin;
