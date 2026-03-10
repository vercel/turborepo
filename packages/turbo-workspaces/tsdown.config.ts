import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/index.ts", "src/cli.ts"],
  format: ["cjs", "esm"],
  dts: false,
  minify: true,
  noExternal: ["fs-extra"],
  outExtensions: ({ format }) => ({
    js: format === "cjs" ? ".js" : ".mjs"
  })
});
