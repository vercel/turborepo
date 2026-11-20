import { defineConfig } from "tsdown";

export default defineConfig({
  entry: {
    jest: "src/jest.ts",
    "jest/filter": "src/jest/filter.ts",
    vitest: "src/vitest.ts"
  },
  format: ["cjs"],
  dts: false,
  outExtensions: () => ({
    js: ".js"
  })
});
