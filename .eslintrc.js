module.exports = {
  root: true,
  extends: ["next", "prettier"],
  settings: {
    next: {
      rootDir: ["docs/", "create-turbo/"],
    },
  },
  rules: {
    "@next/next/no-html-link-for-pages": "off",
  },
  overrides: [
    {
      files: "crates/*/js/**",
      rules: {
        "prefer-const": "error",
        // we can't use `no-var` because it doesn't understand that
        // `declare var` declares a global, but `declare let` does not
        "no-restricted-syntax": [
          "error",
          {
            selector: "VariableDeclaration[kind='var'][declare!=true]",
            message: "Unexpected var, use let or const instead.",
          },
        ],
      },
    },
  ],
};
