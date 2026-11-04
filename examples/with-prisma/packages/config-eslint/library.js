const babelParser = require("@babel/eslint-parser");
const prettierConfig = require("eslint-config-prettier");
const turboConfigModule = require("eslint-config-turbo/flat");

const turboConfig = turboConfigModule.default ?? turboConfigModule;

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  {
    ignores: ["dist/**", "generated/**", ".next/**", "next-env.d.ts"],
  },
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          babelrc: false,
          configFile: false,
          parserOpts: {
            plugins: ["typescript", "jsx"],
          },
        },
      },
    },
  },
  ...turboConfig,
  prettierConfig,
];
