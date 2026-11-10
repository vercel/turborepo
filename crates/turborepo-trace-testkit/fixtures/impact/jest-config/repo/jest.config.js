/** @type {import('jest').Config} */
module.exports = {
  testEnvironment: 'node',
  transform: { '^.+\\.tsx?$': ['ts-jest', { isolatedModules: true }] },
  setupFiles: ['<rootDir>/test/setup.ts'],
  moduleNameMapper: { '^~/config$': '<rootDir>/src/config.dev.ts' },
};
