module.exports = {
  root: true,
  extends: ["next", "prettier"],
  settings: {
    next: {
      rootDir: ["apps/*/", "packages/*/"],
    },
  },
};
