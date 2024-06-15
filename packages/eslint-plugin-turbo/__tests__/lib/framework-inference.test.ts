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
        "../../__fixtures__/framework-inference/apps/nextjs/file.js"
      ),
    },
    {
      code: ` const env2 = process.env.NEXT_PUBLIC_FOO; `,
      options: [{ cwd: path.join(__dirname, "../../__fixtures__/") }],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/nextjs/file.js"
      ),
    },
    {
      code: ` const {NEXT_PUBLIC_FOO} = process.env; `,
      options: [{ cwd: path.join(__dirname, "../../__fixtures__/") }],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/nextjs/file.js"
      ),
    },
  ],
  invalid: [],
});
moduleRuleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `
        const env2 = import.meta.env['NEXT_PUBLIC_FOO'];
      `,
      options: [
        { cwd: path.join(__dirname, "../../__fixtures__/framework-inference") },
      ],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/nextjs/file.js"
      ),
    },
  ],

  invalid: [
    {
      code: `
        const env2 = import.meta.env['ENV_3'];
      `,
      options: [
        { cwd: path.join(__dirname, "../../__fixtures__/framework-inference") },
      ],
      filename: path.join(
        __dirname,
        "../../__fixtures__/framework-inference/apps/vite/invalid.js"
      ),
      errors: [
        {
          message: "ENV_3 is not listed as a dependency in turbo.json",
        },
      ],
    },
  ],
});
