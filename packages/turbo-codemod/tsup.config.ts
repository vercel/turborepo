import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/cli.ts", "src/transforms/*.ts"],
  format: ["esm"],
  shim: true,
  clean: true,
  // minify: true,
  ...options,
}));
