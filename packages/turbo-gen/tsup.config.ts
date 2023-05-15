import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/cli.ts", "src/types.ts"],
  format: ["cjs"],
  dts: true,
  clean: true,
  minify: true,
  ...options,
}));
