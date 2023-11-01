import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/cli.ts"],
  format: ["esm"],
  shim: true, // Add shims for things like __filename and __dirname usage
  clean: true,
  minify: true,
  ...options,
}));
