import eslint from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import turboConfig from "eslint-config-turbo/flat";
import { defineConfig, globalIgnores } from "eslint/config";
import tseslint from "typescript-eslint";

export const libraryConfig = defineConfig(
  eslint.configs.recommended,
  tseslint.configs.recommended,
  turboConfig,
  eslintConfigPrettier,
  globalIgnores(["dist/**"]),
);
