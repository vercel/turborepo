const { copyFileSync, mkdirSync } = require("node:fs");

if (!process.env.DOCS_PREVIEW_TOKEN) {
  throw new Error("DOCS_PREVIEW_TOKEN is required to build the docs preview");
}
mkdirSync("dist", { recursive: true });
copyFileSync("src/index.html", "dist/index.html");
