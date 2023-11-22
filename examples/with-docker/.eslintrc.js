module.exports = {
  root: true,
  extends: ["@repo/eslint-config/index.js"],
  settings: {
    next: {
      rootDir: ["apps/*/"],
    },
  },
};
