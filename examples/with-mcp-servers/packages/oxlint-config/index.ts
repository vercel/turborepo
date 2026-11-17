import { defineConfig } from "oxlint";

export default defineConfig({
  plugins: ["typescript", "oxc"],
  categories: {
    correctness: "error",
  },
  rules: {
    "no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    "typescript/no-explicit-any": "error",
  },
});
