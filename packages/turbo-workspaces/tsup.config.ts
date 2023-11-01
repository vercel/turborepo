import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/index.ts", "src/cli.ts"],
  format: ["esm"],
  shim: true, // Add shims for things like __filename and __dirname usage
  dts: true,
  clean: true,
  minify: true,
  ...options,
}));
