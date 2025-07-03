import type { Linter } from "eslint";
import plugin from "eslint-plugin-turbo";

// eslint-disable-next-line import/no-default-export -- Matching old module.exports
export default [
  {
    plugins: {
      turbo: plugin,
    },
    rules: {
      "turbo/no-undeclared-env-vars": "error",
    },
  },
] satisfies Array<Linter.FlatConfig>;
