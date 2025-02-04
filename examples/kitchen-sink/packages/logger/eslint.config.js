import { config } from "@repo/eslint-config";

/** @type {import("eslint").Linter.Config} */
export default [
  ...config,
  {
    rules: {
      "no-console": "off",
    },
  },
];
