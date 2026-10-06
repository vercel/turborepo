import js from "@eslint/js";
import babelParser from "@babel/eslint-parser";
import turbo from "eslint-config-turbo/flat";
import globals from "globals";

export default [
  { ignores: ["**/node_modules/**", "**/dist/**", "**/storybook-static/**"] },
  js.configs.recommended,
  ...turbo,
  { languageOptions: { globals: globals.node } },
  {
    files: ["**/*.{ts,tsx,mts,cts}"],
    languageOptions: {
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          babelrc: false,
          configFile: false,
          parserOpts: { plugins: ["typescript"] },
        },
      },
    },
    // The check-types task checks undefined names and unused declarations.
    rules: { "no-undef": "off", "no-unused-vars": "off" },
  },
  {
    files: ["**/*.tsx"],
    languageOptions: {
      parserOptions: {
        babelOptions: { parserOpts: { plugins: ["typescript", "jsx"] } },
      },
    },
  },
];
