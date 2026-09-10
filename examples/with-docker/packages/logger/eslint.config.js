const config = require("@repo/eslint-config/react-internal.js");

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  {
    ignores: ["**/__tests__/**"],
  },
  ...config,
];
