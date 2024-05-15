module.exports = {
  extends: ["@repo/eslint-config/library.js"],
  overrides: [
    {
      files: ["*.entity.ts", "*.repository.ts", "*.service.ts"],
    },
  ],
};
