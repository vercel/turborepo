import js from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import turboConfig from "eslint-plugin-turbo";
import tseslint from "typescript-eslint";

/**
 * A shared ESLint configuration for the repository.
 *
 * @type {import("eslint").Linter.Config}
 * */
export const config = [
  js.configs.recommended,
  eslintConfigPrettier,
  ...tseslint.configs.recommended,
  {
    ignores: ["dist/**"],
  },
  {
    plugins: {
      turbo: turboConfig,
    },
    rules: {
      "turbo/no-undeclared-env-vars": "warn",
    },
  },
];
