import globals from "globals";
import pluginReactHooks from "eslint-plugin-react-hooks";
import pluginReactRefresh from "eslint-plugin-react-refresh";
import { config as baseConfig } from "./base.js";

/**
 * A custom ESLint configuration for React applications built with Vite.
 *
 * @type {import("eslint").Linter.Config[]}
 * */
export const viteConfig = [
  ...baseConfig,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        ...globals.browser,
      },
    },
  },
  pluginReactHooks.configs.flat.recommended,
  pluginReactRefresh.configs.vite,
];
