const { defineConfig } = require("eslint/config");
const next = require("@next/eslint-plugin-next");
const base = require("./library.js");

module.exports = defineConfig(base, {
  files: ["**/*.{ts,tsx}"],
  plugins: { "@next/next": next },
  rules: {
    ...next.configs.recommended.rules,
    ...next.configs["core-web-vitals"].rules,
  },
});
