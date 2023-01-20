/** @type {import('ts-jest/dist/types').InitialOptionsTsJest} */
module.exports = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  transformIgnorePatterns: ["/node_modules/(?!(ansi-regex)/)"],
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  testPathIgnorePatterns: ["/__fixtures__/", "/__tests__/test-utils.ts"],
  coveragePathIgnorePatterns: ["/__fixtures__/", "/__tests__/test-utils.ts"],
  collectCoverage: true,
};
