/** @type {import('jest').Config} */
const config = {
  preset: "@turbo/test-utils",
  roots: ["<rootDir>"],
  testPathIgnorePatterns: ["/__fixtures__/"],
  coveragePathIgnorePatterns: ["/__fixtures__/"],
  moduleFileExtensions: ["ts", "tsx", "js", "jsx", "json", "node"],
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
};

module.exports = config;
