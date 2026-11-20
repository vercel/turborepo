import nextPlugin from "@next/eslint-plugin-next";
import { defineConfig } from "eslint/config";
import prettier from "eslint-config-prettier/flat";
import { config as baseConfig } from "./base.js";

export const config = defineConfig([
  baseConfig,
  {
    ...nextPlugin.configs["core-web-vitals"],
    files: ["**/*.{js,jsx,mjs,cjs,ts,tsx,mts,cts}"],
  },
  prettier,
]);
