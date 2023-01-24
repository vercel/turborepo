/** @type {import('ts-jest/dist/types').InitialOptionsTsJest} */
module.exports = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  transformIgnorePatterns: ["/node_modules/(?!(ansi-regex)/)"],
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  testPathIgnorePatterns: ["/__fixtures__/"],
  coveragePathIgnorePatterns: ["/__fixtures__/"],
  collectCoverage: true,
  coverageThreshold: {
    global: {
      branches: 80,
      functions: 89,
      lines: 89,
      statements: 89,
    },
  },
};
