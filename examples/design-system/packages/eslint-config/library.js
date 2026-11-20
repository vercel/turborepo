import babelParser from "@babel/eslint-parser";
import js from "@eslint/js";
import turbo from "eslint-plugin-turbo";
import globals from "globals";

export default [
  { ignores: ["**/dist/**", "**/storybook-static/**", "**/.turbo/**"] },
  js.configs.recommended,
  turbo.configs["flat/recommended"],
  { languageOptions: { globals: globals.node } },
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          babelrc: false,
          configFile: false,
          parserOpts: { plugins: ["typescript", "jsx"] },
        },
      },
    },
    // TypeScript 7 checks names and unused declarations without a compiler-API plugin.
    rules: {
      "no-undef": "off",
      "no-unused-vars": "off",
    },
  },
];
