import path from "node:path";
import { RuleTester } from "eslint";
import { RULES } from "../../../../lib/constants";
import rule from "../../../../lib/rules/no-undeclared-env-vars";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020, sourceType: "module" },
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
        const { TASK_ENV_KEY, ANOTHER_ENV_KEY } = import.meta.env;
      `,
      ...options(),
    },
    {
      code: `
        const { NEW_STYLE_ENV_KEY, TASK_ENV_KEY } = import.meta.env;
      `,
      ...options(),
    },
    {
      code: `
        const { NEW_STYLE_GLOBAL_ENV_KEY, TASK_ENV_KEY } = import.meta.env;
      `,
      ...options(),
    },
    {
      code: `
        const val = import.meta.env["NEW_STYLE_GLOBAL_ENV_KEY"];
      `,
      ...options(),
    },
    {
      code: `
        const { TASK_ENV_KEY, ANOTHER_ENV_KEY } = import.meta.env;
      `,
      ...options(),
    },
    {
      code: `
        const x = import.meta.env.GLOBAL_ENV_KEY;
        const { TASK_ENV_KEY, GLOBAL_ENV_KEY: renamedX } = import.meta.env;
      `,
      ...options(),
    },
    {
      code: "var x = import.meta.env.GLOBAL_ENV_KEY;",
      ...options(),
    },
    {
      code: "let x = import.meta.env.TASK_ENV_KEY;",
      ...options(),
    },
    {
      code: "const x = import.meta.env.ANOTHER_KEY_VALUE;",
      ...options({
        allowList: ["^ANOTHER_KEY_[A-Z]+$"],
      }),
    },
    {
      code: `
        var x = import.meta.env.ENV_VAR_ONE;
        var y = import.meta.env.ENV_VAR_TWO;
      `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
        var x = import.meta.env.ENV_VAR_ONE;
        var y = import.meta.env.ENV_VAR_TWO;
      `,
      ...options({
        allowList: ["^ENV_VAR_O[A-Z]+$", "ENV_VAR_TWO"],
      }),
    },
    {
      code: `
        var globalOrTask = import.meta.env.TASK_ENV_KEY || import.meta.env.GLOBAL_ENV_KEY;
        var oneOrTwo = import.meta.env.ENV_VAR_ONE || import.meta.env.ENV_VAR_TWO;
      `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
        () => { return import.meta.env.GLOBAL_ENV_KEY }
        () => { return import.meta.env.TASK_ENV_KEY }
        () => { return import.meta.env.ENV_VAR_ALLOWED }
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
        function test1(arg1 = import.meta.env.GLOBAL_ENV_KEY) {};
        function test2(arg1 = import.meta.env.TASK_ENV_KEY) {};
        function test3(arg1 = import.meta.env.ENV_VAR_ALLOWED) {};
      `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: `
        (arg1 = import.meta.env.GLOBAL_ENV_KEY) => {}
        (arg1 = import.meta.env.TASK_ENV_KEY) => {}
        (arg1 = import.meta.env.ENV_VAR_ALLOWED) => {}
      `,
      ...options({
        allowList: ["^ENV_VAR_[A-Z]+$"],
      }),
    },
    {
      code: "const getEnv = (key) => import.meta.env[key];",
      ...options(),
    },
    {
      code: "function getEnv(key) { return import.meta.env[key]; }",
      ...options(),
    },
    {
      code: "for (let x of ['ONE', 'TWO', 'THREE']) { console.log(import.meta.env[x]); }",
      ...options(),
    },
  ],

  invalid: [
    {
      code: "let { X } = import.meta.env;",
      ...options(),
      errors: [{ message: "X is not listed as a dependency in turbo.json" }],
    },
    {
      code: "const { X, Y, Z } = import.meta.env;",
      ...options(),
      errors: [
        { message: "X is not listed as a dependency in turbo.json" },
        { message: "Y is not listed as a dependency in turbo.json" },
        { message: "Z is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "const { X, Y: NewName, Z } = import.meta.env;",
      ...options(),
      errors: [
        { message: "X is not listed as a dependency in turbo.json" },
        { message: "Y is not listed as a dependency in turbo.json" },
        { message: "Z is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "var x = import.meta.env.NOT_THERE;",
      ...options(),
      errors: [
        {
          message: "NOT_THERE is not listed as a dependency in turbo.json",
        },
      ],
    },
    {
      code: "var x = import.meta.env.KEY;",
      ...options({
        allowList: ["^ANOTHER_KEY_[A-Z]+$"],
      }),
      errors: [{ message: "KEY is not listed as a dependency in turbo.json" }],
    },
    {
      code: `
        var globalOrTask = import.meta.env.TASK_ENV_KEY_NEW || import.meta.env.GLOBAL_ENV_KEY_NEW;
        var oneOrTwo = import.meta.env.ENV_VAR_ONE || import.meta.env.ENV_VAR_TWO;
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
        () => { return import.meta.env.GLOBAL_ENV_KEY_NEW }
        () => { return import.meta.env.TASK_ENV_KEY_NEW }
        () => { return import.meta.env.ENV_VAR_NOT_ALLOWED }
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
        function test1(arg1 = import.meta.env.GLOBAL_ENV_KEY_NEW) {};
        function test2(arg1 = import.meta.env.TASK_ENV_KEY_NEW) {};
        function test3(arg1 = import.meta.env.ENV_VAR_NOT_ALLOWED) {};
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
        (arg1 = import.meta.env.GLOBAL_ENV_KEY_NEW) => {}
        (arg1 = import.meta.env.TASK_ENV_KEY_NEW) => {}
        (arg1 = import.meta.env.ENV_VAR_NOT_ALLOWED) => {}
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
