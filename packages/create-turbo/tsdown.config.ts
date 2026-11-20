import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/cli.ts"],
  format: ["cjs"],
  dts: false,
  minify: true,
  outExtensions: () => ({
    js: ".js"
  })
});
