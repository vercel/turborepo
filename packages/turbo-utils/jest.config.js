/** @type {import('jest').Config} */
const config = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  transformIgnorePatterns: [
    "node_modules/(?!\\.pnpm/(ansi-regex|@manypkg)|ansi-regex|@manypkg/)"
  ],
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
};

module.exports = config;
