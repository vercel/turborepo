const { extname } = require("node:path");
const { transformSync } = require("esbuild");

module.exports = {
  process(sourceText, sourcePath) {
    const extension = extname(sourcePath);
    const loader = extension === ".tsx" ? "tsx" : "ts";
    const { code, map } = transformSync(sourceText, {
      format: "cjs",
      loader,
      sourcemap: "inline",
      sourcefile: sourcePath,
      target: "node24",
    });

    return { code, map };
  },
};
