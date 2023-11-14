/** @type {import("eslint").Linter.Config} */
module.exports = {
  extends: [require.resolve("@repo/lint/react-internal.js")],
  ignorePatterns: ["turbo/**"],
};
