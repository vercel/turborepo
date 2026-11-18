// Unit tests of the Playwright helpers sit next to the Playwright specs. There is no
// `*.test.ts` file yet, so jest finds nothing; the `*.spec.ts` files are Playwright's.
import type { Config } from 'jest';

const unit: Config = {
  transform: { '^.+\\.ts$': '@swc/jest' },
  testMatch: ['<rootDir>/**/*.test.ts'],
};

export default unit;
