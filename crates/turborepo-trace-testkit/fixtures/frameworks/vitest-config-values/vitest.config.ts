// Aliases that point at directories and files of the project through
// `path.resolve(__dirname, ...)`: resolution rules for the runner, which the config itself
// does not read. One alias resolves with a `customResolver` that is Node's own resolution.
import path from 'node:path';
import { createRequire } from 'node:module';
import { defineConfig } from 'vitest/config';

const require = createRequire(import.meta.url);

export default defineConfig({
  test: {
    environment: 'jsdom',
    setupFiles: ['./test/setup.ts'],
    globals: true,
    alias: [
      {
        find: 'next/font/google',
        replacement: path.resolve(__dirname, './test/mocks/font.ts'),
      },
      {
        find: /^@\/pages\/\(public\)(\/.*)?$/,
        replacement: path.resolve(__dirname, './src/pages/[lang]/(public)$1'),
      },
      {
        find: /^@acme\/docs\/(.+)$/,
        replacement: '@acme/docs/$1',
        customResolver: (request: string): string => require.resolve(request),
      },
      {
        find: '@acme/client',
        replacement: require.resolve('@acme/client', {
          paths: [path.resolve(__dirname, './vendor')],
        }),
      },
      {
        find: '@',
        replacement: path.resolve(__dirname, './src'),
      },
    ],
  },
  resolve: {
    alias: [
      {
        find: '#settings',
        replacement: path.resolve(__dirname, './src/settings.ts'),
      },
    ],
  },
});
