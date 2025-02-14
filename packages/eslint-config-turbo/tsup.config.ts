import { defineConfig, type Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["index.ts", "flat/index.ts"],
  clean: true,
  minify: true,
  dts: true,
  ...options,
}));
