// One testMatch entry comes from a package the evaluator doesn't know, and the entry it can
// read matches no file: the discovery was not read exactly, so it falls back to jest's
// default under rootDir, with an uncertainty, instead of zero tests.
const { extraGlob } = require('@acme/jest-shared');

module.exports = { testMatch: ['<rootDir>/e2e/**/*.e2e.js', extraGlob] };
