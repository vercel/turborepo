const config = require("@repo/eslint-config/library.js");

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  ...config,
  {
    rules: {
      "turbo/no-undeclared-env-vars": [
        "error",
        {
          allowList: ["NODE_ENV"],
        },
      ],
    },
  },
];
