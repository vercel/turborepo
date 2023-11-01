import { defineConfig, type Options } from "tsup";

// eslint-disable-next-line import/no-default-export -- required for tsup
export default defineConfig((options: Options) => ({
  entry: ["src/cli.ts"],
  format: ["esm"],
  shim: true, // Add shims for things like __filename and __dirname usage
  minify: true,
  clean: true,
  ...options,
}));
