/** @type {import("eslint").Linter.Config} */
module.exports = {
  extends: ["@repo/eslint-config/remix.js"],
  parser: "@typescript-eslint/parser",
  parserOptions: {
    project: true,
  },
};
