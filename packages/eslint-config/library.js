const { resolve } = require("node:path");

const project = resolve(process.cwd(), "tsconfig.json");

module.exports = {
  extends: [
    "@vercel/style-guide/eslint/node",
    "@vercel/style-guide/eslint/typescript",
  ].map((config) => require.resolve(config)),
  parserOptions: {
    project: project,
  },
  settings: {
    "import/resolver": {
      typescript: {
        project: project,
      },
    },
  },
  ignorePatterns: ["node_modules/", "dist/"],
  rules: {
    "unicorn/filename-case": ["off"],
    "@typescript-eslint/explicit-function-return-type": ["off"],
  },
};
