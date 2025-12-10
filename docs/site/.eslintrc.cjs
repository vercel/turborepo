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
    // shadcn/ui-style component wrappers around Radix UI primitives
    "components/button.tsx",
    "components/dropdown.tsx",
    "components/popover.tsx",
    "components/nav/navigation-menu.tsx",
    "components/docs-layout/sidebar.tsx",
    // TODO: Need to fix the JSON inference in this file
    "components/examples-table.tsx",
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
        "global-error.tsx",
        "page.tsx",
        "not-found.tsx",
        "source.config.ts",
        "next.config.ts",
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
