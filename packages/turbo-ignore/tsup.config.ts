import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/index.ts"],
  format: ["cjs"],
  minify: true,
  clean: true,
  ...options,
}));
