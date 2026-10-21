const js = require("@eslint/js");
const parser = require("eslint-parser-oxc");
const prettier = require("eslint-config-prettier/flat");
const turbo = require("eslint-config-turbo/flat").default;
const globals = require("globals");

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  {
    ignores: [
      "node_modules/**",
      "dist/**",
      "build/**",
      ".next/**",
      ".cache/**",
      "public/**",
      "**/gatsby-types.d.ts",
      "next-env.d.ts",
    ],
  },
  js.configs.recommended,
  {
    files: ["**/*.{js,jsx,ts,tsx}"],
    languageOptions: {
      parser,
      globals: { ...globals.node, ...globals.browser },
    },
  },
  {
    files: ["**/*.js"],
    languageOptions: { sourceType: "commonjs" },
  },
  ...turbo,
  prettier,
];
