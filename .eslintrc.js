module.exports = {
  root: true,
  extends: ["next", "prettier"],
  settings: {
    next: {
      rootDir: ["docs/", "create-turbo/"],
    },
  },
};
