// testMatch with plain groups (`(test|spec)`), which picomatch reads as alternatives, and a
// negated pattern that leaves the integration tests out.
module.exports = {
  testMatch: [
    '**/__tests__/**/*.(test|spec).[jt]s?(x)',
    '**/*.(test|spec).[jt]s?(x)',
    '!**/__tests__/integration/**',
  ],
};
