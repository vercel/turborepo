const fs = require("node:fs");
const path = require("node:path");

const file = path.join(__dirname, "..", "src", "message.js");
const source = fs.readFileSync(file, "utf8");

if (!source.includes("welcome to mixed providers")) {
  throw new Error("message.js must include the mixed-providers marker text");
}

console.log("web lint check passed");
