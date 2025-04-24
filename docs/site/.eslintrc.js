/** @type {import('eslint').Linter.Config} */
module.exports = {
  root: true,
  extends: ["@turbo/eslint-config/library", "plugin:@next/next/recommended"],
  ignorePatterns: [
    "turbo",
    ".map.ts",
    "!app/.well-known/vercel/flags/route.ts",
    ".source",
    "components/ui/**",
  ],
  overrides: [
    {
      files: ["scripts/**"],
      rules: {
        "no-console": "off",
      },
    },
    {
      files: [
        "next.config.mjs",
        "global-error.jsx",
        "page.tsx",
        "not-found.tsx",
        "layout.tsx",
      ],
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
