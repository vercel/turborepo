import path from "node:path";
import { RuleTester } from "eslint";
import { RULES } from "../../lib/constants";
import rule from "../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020 },
});

const moduleRuleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020, sourceType: "module" },
});

ruleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: ` const env2 = process.env['NEXT_PUBLIC_FOO']; `,
      options: [{ cwd: path.join(__dirname, "../../__fixtures__/") }],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/nextjs/index.js"
      ),
    },
    {
      code: ` const env2 = process.env.NEXT_PUBLIC_FOO; `,
      options: [{ cwd: path.join(__dirname, "../../__fixtures__/") }],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/nextjs/index.js"
      ),
    },
    {
      code: ` const {NEXT_PUBLIC_FOO} = process.env; `,
      options: [{ cwd: path.join(__dirname, "../../__fixtures__/") }],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/nextjs/index.js"
      ),
    },
  ],
  invalid: [],
});
moduleRuleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `
        const env2 = import.meta.env['ENV_2'];
      `,
      options: [
        { cwd: path.join(__dirname, "../../__fixtures__/workspace-configs") },
      ],
      filename: path.join(
        __dirname,
        "../../__fixtures__/workspace-configs/apps/web/index.js"
      ),
    },
  ],

  invalid: [
    {
      code: `
        const env2 = import.meta.env['ENV_3'];
      `,
      options: [
        { cwd: path.join(__dirname, "../../__fixtures__/workspace-configs") },
      ],
      filename: path.join(
        __dirname,
        "../../__fixtures__/workspace-configs/apps/web/index.js"
      ),
      errors: [
        {
          message:
            "ENV_3 is not listed as a dependency in the root turbo.json or workspace (apps/web) turbo.json",
        },
      ],
    },
  ],
});
