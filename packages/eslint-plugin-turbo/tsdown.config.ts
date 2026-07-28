import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["lib/index.ts"],
  format: ["cjs"],
  minify: true,
  dts: false,
  outExtensions: () => ({
    js: ".js"
  })
});
