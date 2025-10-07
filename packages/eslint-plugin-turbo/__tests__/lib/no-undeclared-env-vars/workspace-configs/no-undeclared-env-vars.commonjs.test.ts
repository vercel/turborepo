import path from "node:path";
import { RuleTester } from "eslint";
import { afterEach } from "@jest/globals";
import { RULES } from "../../../../lib/constants";
import rule, { clearCache } from "../../../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020 },
});

const cwd = path.join(__dirname, "../../../../__fixtures__/workspace-configs");
const webFilename = path.join(cwd, "/apps/web/index.js");
const docsFilename = path.join(cwd, "/apps/docs/index.js");
const options = (extra: Record<string, unknown> = {}) => ({
  options: [
    {
      cwd,
      ...extra,
    },
  ],
});

afterEach(() => {
  clearCache();
});

ruleTester.run(RULES.noUndeclaredEnvVars, rule, {
  valid: [
    {
      code: `
          const env2 = process.env['ENV_2'];
        `,
      ...options(),
      filename: webFilename,
    },
    {
      code: `
          const env2 = process.env["ENV_2"];
        `,
      ...options(),
      filename: webFilename,
    },
    {
      code: `
          const { ENV_2 } = process.env;
        `,
      ...options(),
      filename: webFilename,
    },
    {
      code: `
          const { ROOT_DOT_ENV, WEB_DOT_ENV } = process.env;
        `,
      ...options(),
      filename: webFilename,
    },
    {
      code: `
          const { NEXT_PUBLIC_HAHAHAHA } = process.env;
        `,
      ...options(),
      filename: webFilename,
    },
    {
      code: `
          const { ENV_1 } = process.env;
        `,
      ...options(),
      filename: webFilename,
    },
    {
      code: `
          const { CI } = process.env;
        `,
      ...options(),
      filename: webFilename,
    },
  ],
  invalid: [
    {
      code: `
        const env2 = process.env['ENV_3'];
      `,
      ...options(),
      filename: webFilename,
      errors: [
        {
          message:
            "ENV_3 is not listed as a dependency in the root turbo.json or workspace (apps/web) turbo.json",
        },
      ],
    },
    {
      code: `
        const env2 = process.env["ENV_3"];
      `,
      ...options(),
      filename: webFilename,
      errors: [
        {
          message:
            "ENV_3 is not listed as a dependency in the root turbo.json or workspace (apps/web) turbo.json",
        },
      ],
    },
    {
      code: `
        const { ENV_2 } = process.env;
      `,
      ...options(),
      filename: docsFilename,
      errors: [
        {
          message:
            "ENV_2 is not listed as a dependency in the root turbo.json or workspace (apps/docs) turbo.json",
        },
      ],
    },
    {
      code: `
        const { NEXT_PUBLIC_HAHAHAHA, NEXT_PUBLIC_EXCLUDE, NEXT_PUBLIC_EXCLUDED } = process.env;
      `,
      ...options(),
      filename: webFilename,
      errors: [
        {
          message:
            "NEXT_PUBLIC_EXCLUDE is not listed as a dependency in the root turbo.json or workspace (apps/web) turbo.json",
        },
        {
          message:
            "NEXT_PUBLIC_EXCLUDED is not listed as a dependency in the root turbo.json or workspace (apps/web) turbo.json",
        },
      ],
    },
  ],
});
