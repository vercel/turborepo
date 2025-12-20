/** @type {import('jest').Config} */
const config = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  transformIgnorePatterns: ["/node_modules/(?!(ansi-regex)/)"],
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1",
};

module.exports = config;
