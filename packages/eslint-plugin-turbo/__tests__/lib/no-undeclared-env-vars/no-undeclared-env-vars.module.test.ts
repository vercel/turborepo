import { RuleTester } from "eslint";
import { RULES } from "../../../lib/constants";
import rule from "../../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020, sourceType: "module" },
});

ruleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `
          const { TZ } = import.meta.env;
        `,
      options: [{ cwd: "/some/random/path" }],
    },
    {
      code: `
        const { ENV_1 } = import.meta.env;
      `,
      options: [{ cwd: "/some/random/path" }],
    },
  ],
  invalid: [],
});
