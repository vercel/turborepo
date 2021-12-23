module.exports = {
  root: true,
  extends: ["next", "prettier"],
  settings: {
    next: {
      rootDir: ["apps/*/", "packages/*/"],
    },
  },
  rules: {
    "no-html-link-for-pages": "off",
  },
};
