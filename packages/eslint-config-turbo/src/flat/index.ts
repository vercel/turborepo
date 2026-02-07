import type { Linter } from "eslint";
import plugin from "eslint-plugin-turbo";

// eslint-disable-next-line import/no-default-export -- Matching old module.exports
const config: Array<Linter.Config> = [
  {
    plugins: {
      turbo: plugin
    },
    rules: {
      "turbo/no-undeclared-env-vars": "error"
    }
  }
];

export default config;
