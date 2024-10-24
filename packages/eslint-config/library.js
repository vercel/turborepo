const { resolve } = require("node:path");

const project = resolve(process.cwd(), "tsconfig.json");

module.exports = {
  extends: [
    "@vercel/style-guide/eslint/node",
    "@vercel/style-guide/eslint/typescript",
  ].map(require.resolve),
  parserOptions: {
    project,
  },
  settings: {
    "import/resolver": {
      typescript: {
        project,
      },
    },
  },
  ignorePatterns: ["node_modules/", "dist/"],
  rules: {
    "unicorn/filename-case": ["off"],
    "@typescript-eslint/explicit-function-return-type": ["off"],
    "@typescript-eslint/array-type": ["error", { default: "generic" }],
    "import/no-extraneous-dependencies": [
      "error",
      { peerDependencies: true, includeTypes: true },
    ],
  },
  overrides: [
    {
      files: ["*.test.ts"],
      rules: {
        "@typescript-eslint/consistent-type-imports": [
          "error",
          {
            disallowTypeAnnotations: false, // this is needed for `jest.mock<typeof import('module')>`
          },
        ],
      },
    },
    {
      files: ["jest.config.ts", "*/jest-config/*.ts", "jest-preset.ts"],
      rules: {
        "import/no-default-export": ["off"],
      },
    },
  ],
};
