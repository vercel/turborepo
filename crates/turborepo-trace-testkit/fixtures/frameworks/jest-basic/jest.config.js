const path = require('path');
const base = require('./jest.base');

const extraSetup = process.env.CI ? ['<rootDir>/test-utils/ci-setup.ts'] : [];

/** @type {import('jest').Config} */
module.exports = {
  ...base,
  displayName: 'basic',
  testEnvironment: 'jsdom',
  setupFiles: ['<rootDir>/test-utils/env.ts', ...extraSetup],
  setupFilesAfterEnv: [
    require.resolve('./jest.setup.ts'),
    ...(base.setupFilesAfterEnv ?? []),
    'jest-extended/all',
  ],
  globalSetup: path.join(__dirname, 'test-utils/global-setup.ts'),
  moduleNameMapper: {
    ...base.moduleNameMapper,
    '\\.(css|less)$': ['<rootDir>/test-utils/style-mock.ts', 'identity-obj-proxy'],
  },
  testPathIgnorePatterns: ['/node_modules/', '<rootDir>/src/legacy/'],
};
