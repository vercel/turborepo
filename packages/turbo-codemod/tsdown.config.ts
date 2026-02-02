import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/cli.ts", "src/transforms/*.ts"],
  format: ["cjs"],
  minify: true,
  outExtensions: () => ({
    js: ".js"
  })
});
