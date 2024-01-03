/** @type {import("eslint").Linter.Config} */
module.exports = {
  root: true,
  extends: ["@turbo/eslint-config/next"],
  parser: "@typescript-eslint/parser",
  parserOptions: {
    project: true,
  },
  ignorePatterns: [
    // Ignore dotfiles
    ".*.js",
    "tailwind.config.js",
    "next.config.js",
    "scripts/**",
  ],
  globals: {
    JSX: true,
    React: true,
    NodeJS: true,
  },
  overrides: [
    {
      files: ["./pages/**", "./turbo/generators/**", "theme.config.tsx"],
      rules: { "import/no-default-export": "off" },
    },
  ],
};
