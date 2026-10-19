import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/index.tsx"],
  banner: "'use client';",
  format: ["cjs", "esm"],
  deps: { neverBundle: [/^react(?:\/|$)/] },
  dts: true,
});
