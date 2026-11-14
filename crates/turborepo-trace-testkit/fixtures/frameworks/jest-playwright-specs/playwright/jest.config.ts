// A jest config for unit tests of the Playwright helpers, next to the Playwright specs.
// No `*.test.ts` file exists yet, so jest finds nothing here; the `*.spec.ts` files belong
// to Playwright.
import type { Config } from 'jest';

const jestConfig: Config = {
  testMatch: ['<rootDir>/**/*.test.ts'],
  transform: {
    '^.+\\.ts$': '@swc/jest',
  },
};

module.exports = jestConfig;
