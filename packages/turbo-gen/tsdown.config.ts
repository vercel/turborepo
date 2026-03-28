import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/cli.ts", "src/types.ts"],
  format: ["cjs"],
  dts: true,
  minify: true,
  outExtensions: () => ({
    js: ".js",
    dts: ".ts"
  })
});
