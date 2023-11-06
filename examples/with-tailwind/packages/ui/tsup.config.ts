import { defineConfig, Options } from "tsup";
import path from "path";
import fs from "fs-extra";

const jsFileScan = (dir: string): string[] => {
  const paths = [];
  const nodes = fs.readdirSync(dir);
  for (const node of nodes) {
    const nodePath = path.join(dir, node);
    const stat = fs.statSync(nodePath);
    if (stat.isDirectory()) {
      paths.push(...jsFileScan(nodePath));
    } else if (
      stat.isFile() &&
      (node.endsWith(".js") || node.endsWith(".mjs"))
    ) {
      paths.push(nodePath);
    }
  }

  return paths;
};

const exportRegex =
  /export([ \n\t]*(?:[^ \n\t\{\}]+[ \n\t]*,?)?(?:[ \n\t]*\{(?:[ \n\t]*[^ \n\t"'\{\}]+[ \n\t]*,?)+\})?[ \n\t]*)from[ \n\t]*(['"])([^'"\n]+)(?:['"])/g;

const injectUseClient = async (filepath: string) => {
  const fileContent = await fs.readFile(filepath, "utf-8");
  if (!fileContent.startsWith('"use client";')) {
    await fs.writeFile(filepath, '"use client";\n' + fileContent);
  }

  const exportPaths = [...fileContent.matchAll(exportRegex)]?.map(
    (exportStatement) => exportStatement[3],
  );

  await Promise.all(
    exportPaths.map(async (exportPath) => {
      const exportFilePath = path.join(
        path.dirname(filepath),
        exportPath.replace(/^\.\//, ""),
      );
      await injectUseClient(exportFilePath);
    }),
  );
};

export default defineConfig((options: Options) => ({
  treeshake: true,
  splitting: true,
  entry: ["src/**/*.tsx"],
  outDir: "dist",
  format: ["esm"],
  dts: true,
  minify: true,
  clean: false,
  external: ["react"],
  ...options,
  async onSuccess() {
    const filepaths = jsFileScan(path.join(__dirname, "dist", "client"));

    await Promise.all(
      filepaths.map(async (filepath) => {
        await injectUseClient(filepath);
      }),
    );
  },
}));
