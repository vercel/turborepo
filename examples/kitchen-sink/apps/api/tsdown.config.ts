import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/**/*", "!src/**/*.test.*"],
  format: ["cjs"],
  outExtensions: () => ({
    js: ".cjs"
  })
});
