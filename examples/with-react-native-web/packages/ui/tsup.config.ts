import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: {
    ".": "src/index.tsx",
  },
  banner: {
    js: "'use client'",
  },
  format: ["cjs", "esm"],
  external: ["react"],
  dts: true,
  ...options,
}));
