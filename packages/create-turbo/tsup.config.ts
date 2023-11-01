import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/cli.ts"],
  format: ["esm"],
  clean: true,
  minify: true,
  ...options,
}));
