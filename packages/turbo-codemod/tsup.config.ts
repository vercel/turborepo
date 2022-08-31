import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/*.ts", "src/transforms/*.ts"],
  format: ["cjs"],
  clean: true,
  ...options,
}));
