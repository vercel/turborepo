module.exports = {
  root: true,
  extends: ["next", "prettier"],
  ignorePatterns: [
    ".yarn",
    "target",
    "dist",
    "node_modules",
    "crates/*/tests",
    "crates/*/benches",
    "packages/create-turbo/templates",
    "packages/turbo-tracing-next-plugin/test/with-mongodb-mongoose",
  ],
  settings: {
    next: {
      rootDir: ["docs/", "create-turbo/"],
    },
  },
  rules: {
    "@next/next/no-html-link-for-pages": "off",
  },
  overrides: [
    {
      files: ["docs/theme.config.js"],
      rules: {
        "react-hooks/rules-of-hooks": "off",
      },
    },
    {
      files: "crates/*/js/**",
      rules: {
        "prefer-const": "error",
      },
    },
  ],
};
