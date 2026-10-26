const { mkdirSync, writeFileSync } = require("node:fs");
const { renderPage } = require("./src/page.cjs");

mkdirSync("dist", { recursive: true });
writeFileSync("dist/index.html", renderPage());
