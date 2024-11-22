const plugin = require("eslint-plugin-turbo");

module.exports = {
  flat: {
    plugins: {
      turbo: plugin,
    },
    rules: {
      "turbo/no-undeclared-env-vars": "error",
    },
  },
  extends: ["plugin:turbo/recommended"],
};
