// A Next.js app's jest config with a resolver module of its own: the default resolver with
// one more export condition for one request (jest.resolver.cjs).
import type { Config } from 'jest';
import nextJest from 'next/jest';

const own: Config = {
  testMatch: ['<rootDir>/**/*.test.{ts,tsx}'],
  setupFilesAfterEnv: ['<rootDir>/jest.setup.ts'],
  testEnvironment: 'jsdom',
  resolver: '<rootDir>/jest.resolver.cjs',
  // the `#/` imports of the app
  moduleNameMapper: { '^#/(.*)$': '<rootDir>/$1' },
};

export default async (): Promise<Config> => {
  const wrapped = (await nextJest({ dir: __dirname })(own)()) as Config;
  return {
    ...wrapped,
    transformIgnorePatterns: (wrapped.transformIgnorePatterns ?? []).filter(
      (p) => !p.includes('node_modules'),
    ),
  };
};
