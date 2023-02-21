/** @type {import('ts-jest/dist/types').InitialOptionsTsJest} */
module.exports = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  testPathIgnorePatterns: ["/__fixtures__/", "/__tests__/test-utils.ts"],
  coveragePathIgnorePatterns: ["/__fixtures__/", "/__tests__/test-utils.ts"],
  transformIgnorePatterns: ["/node_modules/(?!(ansi-regex)/)"],
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  collectCoverage: true,
  coverageThreshold: {
    global: {
      branches: 83,
      functions: 80,
      lines: 86,
      statements: 86,
    },
  },
};
