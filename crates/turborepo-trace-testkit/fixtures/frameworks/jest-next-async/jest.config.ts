// The shape of `apps/web/jest.config.ts` in a large monorepo (M1): an async default export
// wrapping `nextJest({ dir })(customConfig)()`, which then swaps next's SWC transformer for
// a transformer of its own in the resolved config's `transform`.
import nextJest from 'next/jest';
import type { Config } from 'jest';

const customConfig: Config = {
  setupFilesAfterEnv: ['<rootDir>/test/setup.ts'],
  testEnvironment: 'jsdom',
  testMatch: ['./**/*.test.{ts,tsx}'],
  testPathIgnorePatterns: ['/node_modules/', 'test.mocks'],
};

export default async (): Promise<Config> => {
  const config = (await nextJest({ dir: __dirname })(customConfig)()) as Config;
  const finalConfig = {
    ...config,
    transform: Object.fromEntries(
      Object.entries(config.transform ?? {}).map(([pattern, transformer]) => {
        if (
          Array.isArray(transformer) &&
          transformer[0] ===
            require.resolve('next/dist/build/swc/jest-transformer')
        ) {
          return [
            pattern,
            ['<rootDir>/test/compiler-transformer.cjs', transformer[1]],
          ];
        }
        return [pattern, transformer];
      }),
    ),
    transformIgnorePatterns: (config.transformIgnorePatterns ?? []).filter(
      (pattern) => !pattern.includes('node_modules'),
    ),
  };
  return finalConfig;
};
