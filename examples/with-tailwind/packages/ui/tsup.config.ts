import { defineConfig, Options } from "tsup";
import path from "path";
import fs from "fs-extra";

const jsFileScan = (dir:string):string[] => {
  
  const paths = []
  const nodes = fs.readdirSync(dir);
  for (const node of nodes) {
    const nodePath = path.join(dir, node);
    const stat = fs.statSync(nodePath);
    if (stat.isDirectory()) {
      paths.push(...jsFileScan(nodePath));
    } else if (stat.isFile() && (node.endsWith(".js") || node.endsWith(".mjs"))) {
      paths.push(nodePath);
    }
  }

  return paths;
}

export default defineConfig((options: Options) => ({
  treeshake: true,
  splitting: true,
  entry: ["src/**/*.tsx"],
  outDir: 'dist',
  format: ['cjs'],
  dts: true,
  minify: true,
  clean: true,
  external: ["react"],
  ...options,
  async onSuccess() {
    console.log("tsup onSuccess...")
    await new Promise(resolve => setTimeout(resolve, 1000));
    console.log("tsup onSuccess after 1000ms...")

    const paths = jsFileScan(path.join(__dirname, "dist", "client"))

    // console.log({paths})

    await Promise.all(paths.map(async (file) => {
      await fs.writeFile(
        file,
        '"use client";\n' + await fs.readFile(file)
      );
    }));

    
    
    // fs.writeFileSync(
    //   path.join(__dirname, "dist", "client", "index.mjs"),
    //   '"use client";\n' + await fs.readFile(path.join(__dirname, "dist", "client", "index.mjs"))
    // );

    // move /dist/server/client to /dist/client
    // try {
    //   fs.moveSync(path.join(__dirname, "dist", "server", "client"), path.join(__dirname, "dist", "client"), { overwrite: true })
      
    // } catch (err) {
    //   console.error(err)
    // }

    
  },
}));