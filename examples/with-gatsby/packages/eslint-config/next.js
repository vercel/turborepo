const base = require("./library.js");
const next = require("@next/eslint-plugin-next").default;

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  ...base,
  {
    files: ["**/*.{js,jsx,ts,tsx}"],
    plugins: { "@next/next": next },
    rules: {
      ...next.configs.recommended.rules,
      ...next.configs["core-web-vitals"].rules,
    },
  },
];
