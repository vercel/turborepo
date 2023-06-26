import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/index.ts", "src/cli.ts"],
  format: ["cjs"],
  dts: true,
  clean: true,
  minify: true,
  ...options,
}));
