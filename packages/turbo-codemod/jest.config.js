/** @type {import('jest').Config} */
const config = {
  preset: "@turbo/test-utils",
  testEnvironment: "node",
  transformIgnorePatterns: ["node_modules/*", "packages/turbo-workspaces/*"],
  modulePathIgnorePatterns: [
    "<rootDir>/node_modules",
    "<rootDir>/dist",
    "<rootDir>/__tests__/__fixtures__"
  ],
  testPathIgnorePatterns: [
    "__tests__/__fixtures__/",
    "/__tests__/test-utils.ts"
  ],
  coveragePathIgnorePatterns: [
    "__tests__/__fixtures__/",
    "/__tests__/test-utils.ts"
  ],
  collectCoverage: true,
  coverageThreshold: {
    global: {
      branches: 84,
      functions: 93,
      lines: 92,
      statements: 92
    }
  },
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
};

module.exports = config;
