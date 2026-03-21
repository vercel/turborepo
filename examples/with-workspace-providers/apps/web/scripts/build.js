const fs = require("node:fs");
const path = require("node:path");
const { welcomeMessage } = require("../src/message");

const distDir = path.join(__dirname, "..", "dist");
fs.mkdirSync(distDir, { recursive: true });
fs.writeFileSync(path.join(distDir, "build.txt"), welcomeMessage("builder"));

console.log("web build complete");
