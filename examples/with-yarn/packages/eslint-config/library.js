const eslint = require("@eslint/js");
const { defineConfig, globalIgnores } = require("eslint/config");
const prettier = require("eslint-config-prettier/flat");
const { default: turbo } = require("eslint-config-turbo/flat");

module.exports = defineConfig([
  // Oxlint checks TypeScript sources without depending on the TypeScript JS API.
  globalIgnores(["**/*.ts", "**/*.tsx", "dist/**", ".next/**"]),
  eslint.configs.recommended,
  {
    languageOptions: {
      globals: {
        module: "readonly",
        require: "readonly",
      },
    },
  },
  turbo,
  prettier,
]);
