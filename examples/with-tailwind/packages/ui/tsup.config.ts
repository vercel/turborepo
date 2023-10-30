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
    console.log("tsup onSuccess...")
    await new Promise(resolve => setTimeout(resolve, 1000));
    console.log("tsup onSuccess after 1000ms...")
    // add "use client" banner to /dist/client entry point
    fs.writeFileSync(
      path.join(__dirname, "dist", "server", "client", "index.js"),
      '"use client";\n' + await fs.readFile(path.join(__dirname, "dist", "server", "client", "index.js"))
    );
    fs.writeFileSync(
      path.join(__dirname, "dist", "server", "client", "index.mjs"),
      '"use client";\n' + await fs.readFile(path.join(__dirname, "dist", "server", "client", "index.mjs"))
    );
      
    await new Promise(resolve => setTimeout(resolve, 20000));
    console.log("tsup onSuccess after 20000ms...")

    // move /dist/server/client to /dist/client
    try {
      fs.moveSync(path.join(__dirname, "dist", "server", "client"), path.join(__dirname, "dist", "client"), { overwrite: true })
      
    } catch (err) {
      console.error(err)
    }

    
  },
}));