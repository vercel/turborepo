import { RuleTester } from "eslint";
import { RULES } from "../../../lib/constants";
import rule from "../../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  languageOptions: { ecmaVersion: 2020, sourceType: "script" }
});

ruleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `
        const { TZ } = process.env;
      `,
      options: [{ cwd: "/some/random/path" }]
    },
    {
      code: `
          const { ENV_1 } = process.env;
        `,
      options: [{ cwd: "/some/random/path" }]
    }
  ],
  invalid: []
});
