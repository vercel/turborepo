// Plain groups in testMatch (`(spec|test)`), which picomatch reads as alternatives, and a
// negated pattern that leaves the slow tests out.
module.exports = {
  testMatch: [
    '**/*.(spec|test).[jt]s?(x)',
    '**/__tests__/**/*.(spec|test).[jt]s?(x)',
    '!**/__tests__/slow/**',
  ],
};
