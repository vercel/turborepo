module.exports = {
  extends: ["@turbo/eslint-config/library"],
  overrides: [
    {
      files: ["src/*.ts", "cli/index.cjs"],
      rules: {
        "no-console": "off",
      },
    },
    {
      files: ["src/native.ts", "src/operations.ts"],
      rules: {
        "import/no-default-export": "off",
      },
    },
    {
      files: ["src/*.test.ts"],
      rules: {
        // https://github.com/nodejs/node/issues/51292
        "@typescript-eslint/no-floating-promises": "off",
        "@typescript-eslint/no-unsafe-member-access": "off",
        "@typescript-eslint/no-unsafe-argument": "off",
      },
    },
  ],
};
