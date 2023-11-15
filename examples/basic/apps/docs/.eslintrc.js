/** @type {import("eslint").Linter.Config} */
module.exports = {
  root: true,
  extends: [require.resolve("@repo/lint/next.js")],
  parser: "@typescript-eslint/parser",
  parserOptions: {
    project: true,
  },
};
