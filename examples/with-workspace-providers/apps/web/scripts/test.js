const assert = require("node:assert");
const { welcomeMessage } = require("../src/message");

assert.equal(
  welcomeMessage("friend"),
  "hello friend, welcome to mixed providers",
  "welcome message should be stable"
);

console.log("web tests passed");
