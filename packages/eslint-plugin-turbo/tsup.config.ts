import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["lib/index.ts"],
  clean: true,
  minify: true,
  ...options,
}));
