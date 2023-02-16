const os = require("os");

const { foo } = require("foo");

console.log(foo);

const { existsSync } = eval("require")("fs");

if (__dirname.startsWith(os.tmpdir()) && existsSync("./node_modules/foo/index.js.map")) {
  throw new Error("foo/index.js.map should not exist");
}
