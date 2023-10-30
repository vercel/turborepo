import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  banner: {
    js: `"use client"`,
  },
  treeshake: true,
  splitting: true,
  entry: ["src/index.tsx", "src/client/index.tsx"],
  format: ["esm"],
  dts: true,
  minify: true,
  clean: true,
  external: ["react"],
  ...options,
  
}));
