// The preset is not installed, so its testMatch and roots are unknown, and the roots named
// here hold no test: the discovery was not read exactly, so it falls back to jest's default
// under rootDir, with an uncertainty, instead of zero tests.
module.exports = { preset: '@acme/jest-preset', roots: ['<rootDir>/lib'] };
