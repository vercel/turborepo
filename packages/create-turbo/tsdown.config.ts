import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/cli.ts"],
  format: ["cjs"],
  minify: true,
  outExtensions: () => ({
    js: ".js"
  })
});
