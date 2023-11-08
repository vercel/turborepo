import { defineConfig, Options } from "tsup";

export default defineConfig((options: Options) => ({
  treeshake: true,
  splitting: true,
  entry: ["src/**/*.tsx"],
  // package.json["types"] should match the generated file ext.
  // example: esm -> index.d.mts and cjs -> index.d.ts.
  // problem: eslint `import/recommended` can't resolve package.json["types"] with `.d.mts` (esm only).
  // ref: https://github.com/vercel/turbo/pull/6390
  format: ["esm", "cjs"],
  dts: true,
  minify: true,
  clean: true,
  external: ["react"],
  ...options,
}));
