module.exports = {
  extends: ["@repo/eslint-config/library.js"],
  parser: "@typescript-eslint/parser",
  env: {
    es6: true,
    node: true,
  },
  parserOptions: {
    project: true,
    ecmaVersion: 2020,

  },
  rules: {
    "@typescript-eslint/no-explicit-any": "off",
    'no-undef': 'off',
    "no-unused-vars":'off',
  },
  overrides: [
    {
      files: ["*.entity.ts", "*.repository.ts", "*.service.ts"],
    },
  ],
};
