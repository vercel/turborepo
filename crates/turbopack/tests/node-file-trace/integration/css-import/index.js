const os = require("os");
const { readFileSync } = require('fs')

readFileSync("./style.module.css", 'utf8');

const { existsSync } = eval("require")("fs");

if (__dirname.startsWith(os.tmpdir()) && existsSync("./global.css")) {
  throw new Error("global.css should not exist");
}
