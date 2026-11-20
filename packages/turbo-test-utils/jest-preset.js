const path = require("node:path");

const typescriptCompiler = require.resolve("typescript", {
  paths: [__dirname]
});

/** @type {import('jest').Config} */
module.exports = {
  testEnvironment: "node",
  transform: {
    "^.+\\.tsx?$": [
      require.resolve("ts-jest"),
      {
        compiler: typescriptCompiler,
        isolatedModules: true
      }
    ]
  },
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
};
