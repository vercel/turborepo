/** @type {import('jest').Config} */
const config = {
  preset: "@turbo/test-utils",
  testEnvironment: "node",
  testPathIgnorePatterns: ["/__fixtures__/", "/__tests__/test-utils.ts"],
  coveragePathIgnorePatterns: ["/__fixtures__/", "/__tests__/test-utils.ts"],
  transformIgnorePatterns: ["node_modules/*"],
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  collectCoverage: true,
  coverageThreshold: {
    global: {
      branches: 82,
      functions: 85,
      lines: 92,
      statements: 92
    }
  },
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
};

module.exports = config;
