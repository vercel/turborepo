import { defineConfig, type Options } from "tsup";

// eslint-disable-next-line import/no-default-export -- Default export needed
export default defineConfig((options: Options) => ({
  entry: ["index.ts", "flat/index.ts"],
  clean: true,
  minify: true,
  dts: true,
  ...options,
}));
