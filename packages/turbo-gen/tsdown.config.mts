import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "tsdown";

const __dirname = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  entry: ["src/cli.ts", "src/types.ts"],
  format: ["cjs"],
  dts: true,
  minify: true,
  // Bundle every dependency into the output. CJS/ESM interop is resolved at
  // build time so the module format of individual dependencies is irrelevant.
  // The resulting single-file bundle can then be compiled into a standalone
  // binary via Node.js SEA (see scripts/build-binary.mjs).
  noExternal: [/.*/],
  alias: {
    // @turbo/workspaces publishes built files (dist/) which may not exist
    // during development.  Point directly at the source so the bundler can
    // follow it without a prior build step.
    "@turbo/workspaces": resolve(__dirname, "../turbo-workspaces/src/index.ts")
  },
  outExtensions: () => ({
    js: ".js",
    dts: ".ts"
  })
});
