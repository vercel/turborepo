import "eslint-plugin-only-warn";

import { createRequire } from "node:module";

import babelParser from "@babel/eslint-parser";
import js from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import turboConfig from "eslint-config-turbo/flat";

const require = createRequire(import.meta.url);

/**
 * A shared ESLint configuration for the repository.
 *
 * TypeScript is parsed with Babel instead of typescript-eslint because
 * typescript-eslint requires the legacy TypeScript compiler API, which the
 * native TypeScript 7 compiler no longer provides.
 *
 * @type {import("eslint").Linter.Config[]}
 * */
export const config = [
  js.configs.recommended,
  ...turboConfig,
  eslintConfigPrettier,
  {
    files: ["**/*.ts", "**/*.tsx"],
    languageOptions: {
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          plugins: [require.resolve("@babel/plugin-syntax-jsx")],
          presets: [require.resolve("@babel/preset-typescript")],
        },
      },
    },
  },
  {
    ignores: ["dist/**", ".next/**", "coverage/**"],
  },
];
