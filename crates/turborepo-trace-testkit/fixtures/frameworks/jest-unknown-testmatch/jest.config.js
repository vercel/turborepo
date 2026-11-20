// testMatch comes from a package the evaluator doesn't know: discovery falls back to
// jest's default testMatch under rootDir, with an uncertainty, instead of zero tests.
const { testGlobs } = require('@acme/jest-shared');
module.exports = { testMatch: testGlobs() };
