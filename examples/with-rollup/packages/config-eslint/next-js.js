import pluginNext from "@next/eslint-plugin-next";
import pluginReact from "eslint-plugin-react";
import pluginReactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import { config as baseConfig } from "./base.js";

/**
 * A custom ESLint configuration for apps that use Next.js.
 *
 * @type {import("eslint").Linter.Config[]}
 * */
export const nextJsConfig = [
  ...baseConfig,
  pluginNext.configs["core-web-vitals"],
  pluginReactHooks.configs.flat.recommended,
  {
    plugins: { react: pluginReact },
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
        ...globals.serviceworker,
      },
    },
    rules: {
      // Marks JSX identifiers as used so no-unused-vars works with the
      // Babel parser. The plugin's component rules are not enabled because
      // they are not compatible with ESLint 10 yet.
      "react/jsx-uses-vars": "error",
    },
  },
];
