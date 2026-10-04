const eslint = require("@eslint/js");
const prettierModule = require("eslint-config-prettier/flat");
const turboModule = require("eslint-config-turbo/flat");

const prettier = prettierModule.default ?? prettierModule;
const turbo = turboModule.default ?? turboModule;

module.exports = [
  eslint.configs.recommended,
  {
    languageOptions: {
      globals: {
        module: "readonly",
        require: "readonly",
      },
    },
  },
  ...(Array.isArray(turbo) ? turbo : [turbo]),
  ...(Array.isArray(prettier) ? prettier : [prettier]),
  {
    ignores: ["**/*.ts", "**/*.tsx", "dist/**", "node_modules/**"],
  },
];
