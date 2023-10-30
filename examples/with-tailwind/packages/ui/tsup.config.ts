import { defineConfig, Options } from "tsup";
import path from "path";
import fs from "fs-extra";

export default defineConfig((options: Options) => ({
  treeshake: true,
  splitting: true,
  entry: ["src/index.tsx", "src/client/index.tsx"],
  outDir: 'dist/server',
  format: ['esm', 'cjs'],
  dts: true,
  minify: true,
  clean: true,
  external: ["react"],
  ...options,
  async onSuccess() {

    // add "use client" banner to /dist/client entry point
    fs.writeFileSync(
      path.join(__dirname, "dist", "server", "client", "index.js"),
      '"use client";\n' + await fs.readFile(path.join(__dirname, "dist", "server", "client", "index.js"))
    );
    fs.writeFileSync(
      path.join(__dirname, "dist", "server", "client", "index.mjs"),
      '"use client";\n' + await fs.readFile(path.join(__dirname, "dist", "server", "client", "index.mjs"))
    );
      
    // move /dist/server/client to /dist/client
    try {
      fs.moveSync(path.join(__dirname, "dist", "server", "client"), path.join(__dirname, "dist", "client"), { overwrite: true })
      
    } catch (err) {
      console.error(err)
    }
  },
}));