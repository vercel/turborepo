const library = require("./library.js");

module.exports = [
  ...library,
  {
    ignores: [".next/**"],
  },
];
