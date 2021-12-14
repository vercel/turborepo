module.exports = {
  root: true,
  parser: "@typescript-eslint/parser",
  env: { node: true, jest: true },
  extends: [
    "eslint:recommended",
    "plugin:import/recommended",
    "plugin:import/typescript",
  ],
  plugins: ["import"],
  rules: {
    "no-empty": ["error", { allowEmptyCatch: true }],
    "import/no-named-as-default-member": "off",
  },
  settings: {
    "import/parsers": {
      "@typescript-eslint/parser": [".ts"],
    },
    "import/resolver": {
      typescript: {
        alwaysTryTypes: true,
        project: "./tsconfig.json",
      },
    },
  },
};
