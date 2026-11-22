/** @type {import('jest').Config} */
const config = {
  preset: "@turbo/test-utils",
  testPathIgnorePatterns: ["/dist/", "/\\.trace-fixture-"],
  modulePathIgnorePatterns: [
    "<rootDir>/node_modules",
    "<rootDir>/dist",
    "<rootDir>/\\.trace-fixture-"
  ],
  watchPathIgnorePatterns: ["/\\.trace-fixture-"],
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
};

module.exports = config;
