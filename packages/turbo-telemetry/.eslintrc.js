module.exports = {
  extends: ["@turbo/eslint-config/library"],
  overrides: [
    {
      files: ["src/utils.ts"],
      rules: {
        "import/no-default-export": "off",
      },
    },
    {
      files: ["src/**/*.test.ts"],
      rules: {
        // https://github.com/nodejs/node/issues/51292
        "@typescript-eslint/no-floating-promises": "off",
        "@typescript-eslint/no-unsafe-member-access": "off",
        "@typescript-eslint/no-unsafe-argument": "off",
      },
    },
  ],
};
