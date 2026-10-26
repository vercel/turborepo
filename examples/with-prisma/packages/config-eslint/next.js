const nextPlugin = require("@next/eslint-plugin-next");
const config = require("./library.js");

/** @type {import("eslint").Linter.Config[]} */
module.exports = [...config, nextPlugin.configs["core-web-vitals"]];
