import js from "@eslint/js";
import { FlatCompat } from "@eslint/eslintrc";
import ts from "typescript-eslint";
import eslintConfigPrettier from "eslint-config-prettier";
import eslintPluginOnlyWarn from "eslint-plugin-only-warn";
import path from "path";
import { fileURLToPath } from "url";
import globals from "globals";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const compat = new FlatCompat({
  baseDirectory: __dirname,
});

export default ts.config(
  {
    ignores: [
      // Ignore dotfiles
      ".*.js",
      "node_modules/",
      "dist/",
    ],
  },
  {
    languageOptions: {
      parserOptions: {
        ecmaFeatures: {
          jsx: true,
        },
      },
      globals: {
        ...globals.browser,
      },
    },
  },
  {
    plugins: {
      ["only-warn"]: eslintPluginOnlyWarn,
    },
  },
  js.configs.recommended,
  ...compat.extends("turbo"),
  ...ts.config({
    files: ["**/*.js?(x)", "**/*.ts?(x)"],
    extends: [...ts.configs.recommended],
    languageOptions: {
      parserOptions: {
        projectService: {
          allowDefaultProject: ["*.*s", "turbo/generators/*.*s"],
        },
      },
    },
  }),
  eslintConfigPrettier
);
