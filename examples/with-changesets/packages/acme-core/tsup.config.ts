import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    ".": "src/index.tsx",
  },
  banner: {
    js: "'use client'",
  },
  format: ["cjs", "esm"],
  external: ["react"],
  dts: true,
});
