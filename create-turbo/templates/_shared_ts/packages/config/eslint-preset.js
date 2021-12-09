module.exports = {
  extends: ["next", "prettier"],
  settings: {
    next: {
      rootDir: [
        "apps/docs/",
        "apps/web/",
        "packages/ui/",
        "packages/config/",
        "packages/tsconfig/",
      ],
    },
  },
};
