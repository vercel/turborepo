/** @type {import("eslint").Linter.Config} */
module.exports = {
  extends: ["@repo/eslint-config/gatsby.js"],
  ignorePatterns: ["gatsby-types.d.ts"],
};
