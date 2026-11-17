module.exports = {
  testMatch: ['**/*.test.ts'],
  // One copy of the built package for every test of the app.
  moduleNameMapper: {
    '^@acme/ui$': require.resolve('@acme/ui'),
  },
};
