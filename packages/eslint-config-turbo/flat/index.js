const plugin = require("eslint-plugin-turbo");

module.exports = [
  {
    plugins: {
      turbo: plugin,
    },
    rules: {
      "turbo/no-undeclared-env-vars": "error",
    },
  },
];
