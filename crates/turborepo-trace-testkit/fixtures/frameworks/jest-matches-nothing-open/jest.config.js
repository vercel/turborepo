// A testMatch that matches no file, in a config that was not read exactly (an unknown
// spread may replace it): discovery falls back to jest's default under rootDir, with an
// uncertainty.
const overrides = loadOverrides();

module.exports = { testMatch: ['<rootDir>/e2e/**/*.e2e.js'], ...overrides };
