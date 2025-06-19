import path from "node:path";
import { RuleTester } from "eslint";
import { RULES } from "../../../../lib/constants";
import rule from "../../../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020 },
});

const cwd = path.join(__dirname, "../../../../__fixtures__/configs/single");
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
      code: `
          const { TASK_ENV_KEY, ANOTHER_ENV_KEY } = process.env;
        `,
      ...options(),
    },
    {
      code: `
          const { NEW_STYLE_ENV_KEY, TASK_ENV_KEY } = process.env;
        `,
      ...options(),
    },
    {
      code: `
          const { NEW_STYLE_GLOBAL_ENV_KEY, TASK_ENV_KEY } = process.env;
        `,
      ...options(),
    },
    {
      code: `
          const val = process.env["NEW_STYLE_GLOBAL_ENV_KEY"];
        `,
      ...options(),
    },
    {
      code: `
          const { TASK_ENV_KEY, ANOTHER_ENV_KEY } = process.env;
        `,
      ...options(),
    },
    {
      code: `
          const x = process.env.GLOBAL_ENV_KEY;
          const { TASK_ENV_KEY, GLOBAL_ENV_KEY: renamedX } = process.env;
        `,
      ...options(),
    },
    {
      code: "var x = process.env.GLOBAL_ENV_KEY;",
      ...options(),
    },
    {
      code: "let x = process.env.TASK_ENV_KEY;",
      ...options(),
    },
    {
      code: "const x = process.env.ANOTHER_KEY_VALUE;",
      ...options({
        allowList: ["^ANOTHER_KEY_[A-Z]+$"],
      }),
    },
    {
      code: `
          var x = process.env.ENV_VAR_ONE;
          var y = process.env.ENV_VAR_TWO;
        `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
          var x = process.env.ENV_VAR_ONE;
          var y = process.env.ENV_VAR_TWO;
        `,
      ...options({
        allowList: ["^ENV_VAR_O[A-Z]+$", "ENV_VAR_TWO"],
      }),
    },
    {
      code: `
          var globalOrTask = process.env.TASK_ENV_KEY || process.env.GLOBAL_ENV_KEY;
          var oneOrTwo = process.env.ENV_VAR_ONE || process.env.ENV_VAR_TWO;
        `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
          () => { return process.env.GLOBAL_ENV_KEY }
          () => { return process.env.TASK_ENV_KEY }
          () => { return process.env.ENV_VAR_ALLOWED }
        `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
          var foo = process?.env.GLOBAL_ENV_KEY
          var foo = process?.env.TASK_ENV_KEY
          var foo = process?.env.ENV_VAR_ALLOWED
        `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
          function test(arg1 = process.env.GLOBAL_ENV_KEY) {};
          function test(arg1 = process.env.TASK_ENV_KEY) {};
          function test(arg1 = process.env.ENV_VAR_ALLOWED) {};
        `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
          (arg1 = process.env.GLOBAL_ENV_KEY) => {}
          (arg1 = process.env.TASK_ENV_KEY) => {}
          (arg1 = process.env.ENV_VAR_ALLOWED) => {}
        `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: "const getEnv = (key) => process.env[key];",
      ...options(),
    },
    {
      code: "function getEnv(key) { return process.env[key]; }",
      ...options(),
    },
    {
      code: "for (let x of ['ONE', 'TWO', 'THREE']) { console.log(process.env[x]); }",
      ...options(),
    },
  ],
  invalid: [
    {
      code: "let { X } = process.env;",
      ...options(),
      errors: [{ message: "X is not listed as a dependency in turbo.json" }],
    },
    {
      code: "const { X, Y, Z } = process.env;",
      ...options(),
      errors: [
        { message: "X is not listed as a dependency in turbo.json" },
        { message: "Y is not listed as a dependency in turbo.json" },
        { message: "Z is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "const { X, Y: NewName, Z } = process.env;",
      ...options(),
      errors: [
        { message: "X is not listed as a dependency in turbo.json" },
        { message: "Y is not listed as a dependency in turbo.json" },
        { message: "Z is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "var x = process.env.NOT_THERE;",
      ...options(),
      errors: [
        {
          message: "NOT_THERE is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: "var x = process.env.KEY;",
      ...options({
        allowList: ["^ANOTHER_KEY_[A-Z]+$"],
      }),

      errors: [{ message: "KEY is not listed as a dependency in turbo.json" }],
    },
    {
      code: `
          var globalOrTask = process.env.TASK_ENV_KEY_NEW || process.env.GLOBAL_ENV_KEY_NEW;
          var oneOrTwo = process.env.ENV_VAR_ONE || process.env.ENV_VAR_TWO;
        `,
      ...options(),
      errors: [
        {
          message:
            "TASK_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "GLOBAL_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message: "ENV_VAR_ONE is not listed as a dependency in turbo.json",
        },
        {
          message: "ENV_VAR_TWO is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: `
          () => { return process.env.GLOBAL_ENV_KEY_NEW }
          () => { return process.env.TASK_ENV_KEY_NEW }
          () => { return process.env.ENV_VAR_NOT_ALLOWED }
        `,
      ...options(),
      errors: [
        {
          message:
            "GLOBAL_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "TASK_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "ENV_VAR_NOT_ALLOWED is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: `
          var foo = process?.env.GLOBAL_ENV_KEY_NEW
          var foo = process?.env.TASK_ENV_KEY_NEW
          var foo = process?.env.ENV_VAR_NOT_ALLOWED
        `,
      ...options(),
      errors: [
        {
          message:
            "GLOBAL_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "TASK_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "ENV_VAR_NOT_ALLOWED is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: `
          function test(arg1 = process.env.GLOBAL_ENV_KEY_NEW) {};
          function test(arg1 = process.env.TASK_ENV_KEY_NEW) {};
          function test(arg1 = process.env.ENV_VAR_NOT_ALLOWED) {};
        `,
      ...options(),
      errors: [
        {
          message:
            "GLOBAL_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "TASK_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "ENV_VAR_NOT_ALLOWED is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: `
          (arg1 = process.env.GLOBAL_ENV_KEY_NEW) => {}
          (arg1 = process.env.TASK_ENV_KEY_NEW) => {}
          (arg1 = process.env.ENV_VAR_NOT_ALLOWED) => {}
        `,
      ...options(),
      errors: [
        {
          message:
            "GLOBAL_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "TASK_ENV_KEY_NEW is not listed as a dependency in turbo.json",
        },
        {
          message:
            "ENV_VAR_NOT_ALLOWED is not listed as a dependency in turbo.json",
        },
      ],
    },
  ],
});
