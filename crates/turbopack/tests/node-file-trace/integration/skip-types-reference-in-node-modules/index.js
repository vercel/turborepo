const os = require("os");

const { foo } = require("foo");

console.log(foo);

const { existsSync } = eval("require")("fs");

if (__dirname.startsWith(os.tmpdir()) && existsSync("./node_modules/foo/global.d.ts")) {
  throw new Error("foo/global.d.ts should not exist");
}
