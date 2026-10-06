import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/button.tsx"],
  format: ["cjs", "esm"],
  dts: true,
  platform: "browser",
  target: "es2022",
  deps: { neverBundle: [/^react(?:\/|$)/] },
});
