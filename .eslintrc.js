module.exports = {
  root: true,
  extends: ["next", "prettier"],
  ignorePatterns: [
    ".yarn",
    "target",
    "dist",
    "node_modules",
    "crates",
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
};
