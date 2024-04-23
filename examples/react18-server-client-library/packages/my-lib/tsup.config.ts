import { defineConfig, type Options } from "tsup";
import react18Plugin from "esbuild-plugin-react18";

export default defineConfig(
  (options) =>
    ({
      format: ["cjs", "esm"],
      target: "es2019",
      sourcemap: false,
      dts: true,
      clean: true,
      bundle: true,
      minify: !options.watch,
      esbuildPlugins: [react18Plugin()],
      legacyOutput: true,
    }) as Options,
);
