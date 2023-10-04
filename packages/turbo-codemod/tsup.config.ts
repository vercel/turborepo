import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/cli.ts", "src/transforms/*.ts"],
  format: ["cjs"],
  clean: true,
  minify: true,
  ...options,
}));
