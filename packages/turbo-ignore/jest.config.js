/** @type {import('ts-jest/dist/types').InitialOptionsTsJest} */
module.exports = {
  preset: "ts-jest/presets/js-with-ts",
  testEnvironment: "node",
  modulePathIgnorePatterns: ["<rootDir>/node_modules", "<rootDir>/dist"],
  transformIgnorePatterns: ["/node_modules/(?!(ansi-regex)/)"],
};
