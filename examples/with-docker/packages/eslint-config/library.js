const babelParser = require("@babel/eslint-parser");
const prettierConfig = require("eslint-config-prettier/flat");
const turboConfig = require("eslint-config-turbo/flat").default;

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  {
    ignores: [".next/**", "dist/**", "node_modules/**"],
  },
  ...turboConfig,
  {
    files: ["**/*.{js,jsx,ts,tsx}"],
    languageOptions: {
      ecmaVersion: "latest",
      parser: babelParser,
      parserOptions: {
        babelOptions: {
          plugins: [require.resolve("@babel/plugin-syntax-jsx")],
          presets: [
            [
              require.resolve("@babel/preset-typescript"),
              { ignoreExtensions: true },
            ],
          ],
        },
        requireConfigFile: false,
        sourceType: "module",
      },
      sourceType: "module",
    },
    rules: {
      "no-undef": "off"
    },
  },
  prettierConfig,
];
