// A `|` outside a group: picomatch puts it into its regex as is, and jest runs
// src/a.test.ts with this pattern. The glob translator reads the `|` literally, so the
// pattern is not read exactly: discovery falls back to jest's default under rootDir, with an
// uncertainty, instead of concluding that there are no tests.
module.exports = { testMatch: ['**/*.test.ts|**/*.spec.ts'] };
