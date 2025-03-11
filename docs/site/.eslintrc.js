/** @type {import('eslint').Linter.Config} */
module.exports = {
  root: true,
  extends: ["@turbo/eslint-config/library", "next"],
  ignorePatterns: [
    "turbo",
    ".map.ts",
    "!app/.well-known/vercel/flags/route.ts",
    ".source",
  ],
  overrides: [
    {
      files: ["scripts/**"],
      rules: {
        "no-console": "off",
      },
    },
    {
      files: ["next.config.mjs", "global-error.jsx"],
      rules: {
        "import/no-default-export": "off",
      },
    },
    {
      files: ["source.ts"],
      reportUnusedDisableDirectives: false,
    },
  ],
};
