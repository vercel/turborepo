import type { Config } from 'jest';

const config: Config = {
  displayName: 'a',
  setupFiles: ['./setup-a.ts'],
  moduleNameMapper: { '^~/(.*)$': '<rootDir>/src/$1' },
};

export default config;
