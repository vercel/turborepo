import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/index.ts", "src/cli.ts"],
  format: ["cjs", "esm"],
  dts: true,
  minify: true,
  outExtensions: ({ format }) => ({
    js: format === "cjs" ? ".js" : ".mjs",
    dts: format === "cjs" ? ".ts" : ".mts"
  })
});
