import path from "node:path";
import { RuleTester } from "eslint";
import { RULES } from "../../../../lib/constants";
import rule from "../../../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020, sourceType: "module" },
});

const cwd = path.join(
  __dirname,
  "../../../../__fixtures__/framework-inference"
);
const nextJsFilename = path.join(cwd, "/apps/nextjs/index.js");
const viteFilename = path.join(cwd, "/apps/vite/index.js");
const options = (extra: Record<string, unknown> = {}) => ({
  options: [
    {
      cwd,
      ...extra,
    },
  ],
});

ruleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `const { NEXT_PUBLIC_ZILTOID } = import.meta.env;`,
      ...options(),
      filename: nextJsFilename,
    },
    {
      code: `const { VITE_THINGS } = import.meta.env;`,
      ...options(),
      filename: viteFilename,
    },
  ],
  invalid: [
    {
      code: `const { NEXT_PUBLIC_ZILTOID } = import.meta.env;`,
      ...options(),
      filename: viteFilename,
      errors: [
        {
          message:
            "NEXT_PUBLIC_ZILTOID is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: `const { VITE_THINGS } = import.meta.env;`,
      ...options(),
      filename: nextJsFilename,
      errors: [
        {
          message: "VITE_THINGS is not listed as a dependency in turbo.json",
        },
      ],
    },
  ],
});
