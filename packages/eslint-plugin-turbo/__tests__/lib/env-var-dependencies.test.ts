import { RULES } from "../../lib/constants";
import rule from "../../lib/rules/no-uncached-env-vars";
import { RuleTester } from "eslint";

const ruleTester = new RuleTester({
  parserOptions: { ecmaVersion: 2020 },
});

const getTestTurboConfig = () => {
  return {
    $schema: "./docs/public/schema.json",
    baseBranch: "origin/main",
    globalDependencies: ["$GLOBAL_ENV_KEY"],
    pipeline: {
      test: {
        outputs: ["coverage/**/*"],
        dependsOn: ["^build"],
      },
      lint: {
        outputs: [],
      },
      dev: {
        cache: false,
      },
      build: {
        outputs: ["dist/**/*", ".next/**/*"],
        dependsOn: ["^build", "$TASK_ENV_KEY", "$ANOTHER_ENV_KEY"],
      },
    },
  };
};

ruleTester.run(RULES.noUncachedEnvVars, rule, {
  valid: [
    {
      code: `
        const { TASK_ENV_KEY, ANOTHER_ENV_KEY } = process.env;
      `,
      options: [{ turboConfig: getTestTurboConfig() }],
    },
    {
      code: `
        const x = process.env.GLOBAL_ENV_KEY;
        const { TASK_ENV_KEY, GLOBAL_ENV_KEY: renamedX } = process.env;
      `,
      options: [{ turboConfig: getTestTurboConfig() }],
    },
    {
      code: "var x = process.env.GLOBAL_ENV_KEY;",
      options: [{ turboConfig: getTestTurboConfig() }],
    },
    {
      code: "let x = process.env.TASK_ENV_KEY;",
      options: [{ turboConfig: getTestTurboConfig() }],
    },
    {
      code: "const x = process.env.ANOTHER_KEY_VALUE;",
      options: [
        {
          turboConfig: getTestTurboConfig(),
          allowList: ["^ANOTHER_KEY_[A-Z]+$"],
        },
      ],
    },
    {
      code: `
      var x = process.env.ENV_VAR_ONE;
      var y = process.env.ENV_VAR_TWO;
      `,
      options: [
        {
          turboConfig: getTestTurboConfig(),
          allowList: ["^ENV_VAR_[A-Z]+$"],
        },
      ],
    },
    {
      code: `
      var x = process.env.ENV_VAR_ONE;
      var y = process.env.ENV_VAR_TWO;
      `,
      options: [
        {
          turboConfig: getTestTurboConfig(),
          allowList: ["^ENV_VAR_O[A-Z]+$", "ENV_VAR_TWO"],
        },
      ],
    },
  ],

  invalid: [
    {
      code: "let { X } = process.env;",
      options: [{ turboConfig: getTestTurboConfig() }],
      errors: [{ message: "$X is not listed as a dependency in turbo.json" }],
    },
    {
      code: "const { X, Y, Z } = process.env;",
      options: [{ turboConfig: getTestTurboConfig() }],
      errors: [
        { message: "$X is not listed as a dependency in turbo.json" },
        { message: "$Y is not listed as a dependency in turbo.json" },
        { message: "$Z is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "const { X, Y: NewName, Z } = process.env;",
      options: [{ turboConfig: getTestTurboConfig() }],
      errors: [
        { message: "$X is not listed as a dependency in turbo.json" },
        { message: "$Y is not listed as a dependency in turbo.json" },
        { message: "$Z is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "var x = process.env.NOT_THERE;",
      options: [{ turboConfig: getTestTurboConfig() }],
      errors: [
        { message: "$NOT_THERE is not listed as a dependency in turbo.json" },
      ],
    },
    {
      code: "var x = process.env.KEY;",
      options: [
        {
          turboConfig: getTestTurboConfig(),
          allowList: ["^ANOTHER_KEY_[A-Z]+$"],
        },
      ],
      errors: [{ message: "$KEY is not listed as a dependency in turbo.json" }],
    },
  ],
});
