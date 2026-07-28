const bar = require("./bar");

if (process.env.NODE_ENV === "production") {
  const foo = require("./foo");
  foo();
}

bar();
