import js from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import turboConfig from "eslint-config-turbo/flat";
import solid from "eslint-plugin-solid/configs/typescript";
import globals from "globals";
import tseslint from "typescript-eslint";

/**
 * Shared ESLint flat configuration for Solid + TypeScript packages.
 *
 * typescript-eslint runs against the TypeScript 6 compiler API, which pnpm
 * auto-installs to satisfy its peer range. TypeScript 7 (the native compiler)
 * does not expose a compiler API until TypeScript 7.1, so the workspaces keep
 * their own `typescript` pins for `tsc` while linting stays on the supported
 * TS 6 API, as recommended by the TypeScript team.
 */
export const config = tseslint.config(
  {
    ignores: ["**/dist/**", "**/.output/**", "**/.nitro/**", "**/.turbo/**"],
  },
  js.configs.recommended,
  ...turboConfig,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    ...solid,
  },
  {
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
  },
  eslintConfigPrettier,
);
