import type { Config } from "jest";

const config = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  testPathIgnorePatterns: [
    "__fixtures__/",
    "/__tests__/test-utils.ts",
    "/__tests__/__mocks__/"
  ],
  coveragePathIgnorePatterns: [
    "__fixtures__/",
    "/__tests__/test-utils.ts",
    "/__tests__/__mocks__/"
  ],
  transformIgnorePatterns: [
    "node_modules/(?!\\.pnpm/@manypkg|@manypkg/)",
    "packages/turbo-workspaces/*"
  ],
  moduleNameMapper: {
    "^node-plop$": "<rootDir>/__tests__/__mocks__/node-plop.ts",
    "^inquirer$": "<rootDir>/__tests__/__mocks__/inquirer.ts"
  },
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  collectCoverage: true,
  verbose: process.env.RUNNER_DEBUG === "1",
  silent: process.env.RUNNER_DEBUG !== "1"
} as const satisfies Config;

export default config;
