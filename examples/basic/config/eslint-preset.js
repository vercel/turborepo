module.exports = {
  extends: ["next", "prettier"],
  settings: {
    next: {
      rootDir: ["docs/", "ui/", "web/", "config/", "tsconfig/"],
    },
  },
};
