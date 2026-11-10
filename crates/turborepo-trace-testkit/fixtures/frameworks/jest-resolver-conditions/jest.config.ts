// A Next.js site's jest config with a custom resolver module. The resolver is the default
// resolver with one more export condition for one request (jest.resolver.cjs).
import nextJest from 'next/jest';
import type { Config } from 'jest';

const customConfig: Config = {
  resolver: '<rootDir>/jest.resolver.cjs',
  moduleNameMapper: {
    // Matches paths in tsconfig.json
    '#/(.*)': '<rootDir>/$1',
  },
  testEnvironment: 'jsdom',
  testMatch: ['<rootDir>/**/*.test.{ts,tsx}'],
  setupFilesAfterEnv: ['<rootDir>/jest.setup.ts'],
};

export default async (): Promise<Config> => {
  const config = (await nextJest({ dir: __dirname })(customConfig)()) as Config;

  const finalConfig = {
    ...config,
    transformIgnorePatterns: (config.transformIgnorePatterns ?? []).filter(
      (pattern) => !pattern.includes('node_modules'),
    ),
  };
  return finalConfig;
};
