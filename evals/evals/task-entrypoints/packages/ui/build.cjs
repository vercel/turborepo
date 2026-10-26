const { copyFileSync, mkdirSync } = require("node:fs");

mkdirSync("dist", { recursive: true });
copyFileSync("src/index.cjs", "dist/index.cjs");
