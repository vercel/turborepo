import type { ESLint } from "eslint";
import { name, version } from "../package.json";
import { RULES } from "./constants";
import noUndeclaredEnvVars from "./rules/no-undeclared-env-vars";
import recommended from "./configs/recommended";
import flatRecommended from "./configs/flat/recommended";

const plugin = {
  meta: { name, version },
  rules: {
    [RULES.noUndeclaredEnvVars]: noUndeclaredEnvVars,
  },
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

export const { rules, configs } = plugin;

export default plugin;
