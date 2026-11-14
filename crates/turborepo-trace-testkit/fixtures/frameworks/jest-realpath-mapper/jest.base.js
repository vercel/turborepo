// The repository's shared jest options.
module.exports = {
  testEnvironment: 'node',
  moduleNameMapper: {
    '^@test/(.*)$': '<rootDir>/test/$1',
  },
  setupFilesAfterEnv: [],
};
