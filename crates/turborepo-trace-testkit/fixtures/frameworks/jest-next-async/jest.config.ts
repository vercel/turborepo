// An async default export around `nextJest({ dir })(own)()`, which then replaces next's SWC
// transformer with a transformer of its own in the resolved config's `transform`.
import type { Config } from 'jest';
import nextJest from 'next/jest';

const own: Config = {
  testPathIgnorePatterns: ['/__samples__/'],
  testMatch: ['**/*.test.{ts,tsx}'],
  setupFilesAfterEnv: ['<rootDir>/test/after-env.ts'],
  testEnvironment: 'jsdom',
};

export default async (): Promise<Config> => {
  const wrapped = (await nextJest({ dir: __dirname })(own)()) as Config;
  return {
    ...wrapped,
    transform: Object.fromEntries(
      Object.entries(wrapped.transform ?? {}).map(([glob, entry]) => {
        if (
          Array.isArray(entry) &&
          entry[0] === require.resolve('next/dist/build/swc/jest-transformer')
        ) {
          return [glob, ['<rootDir>/test/swc-wrap.cjs', entry[1]]];
        }
        return [glob, entry];
      }),
    ),
    transformIgnorePatterns: (wrapped.transformIgnorePatterns ?? []).filter(
      (p) => !p.includes('node_modules'),
    ),
  };
};
