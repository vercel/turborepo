import type { Config } from "jest";

const config = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  testPathIgnorePatterns: ["/__fixtures__/"],
  coveragePathIgnorePatterns: ["/__fixtures__/"],
  collectCoverage: true,
  coverageThreshold: {
    global: {
      branches: 100,
      functions: 100,
      lines: 100,
      statements: 100,
    },
  },
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  transformIgnorePatterns: ["node_modules/*"],
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1",
} as const satisfies Config;

export default config;
