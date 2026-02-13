import { defineConfig } from "tsdown";

// DTS generation for the PlopTypes re-export consumed by user generator configs.
// The CLI binary is compiled separately via `bun build --compile`.
export default defineConfig({
  entry: ["src/types.ts"],
  format: ["cjs"],
  dts: true,
  outExtensions: () => ({
    js: ".js",
    dts: ".ts"
  })
});
