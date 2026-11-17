// An app's vitest config whose aliases point at directories and files of the app with
// `path.resolve(__dirname, ...)`. The targets are resolution rules for the runner: the
// config does not read them. One alias resolves through a `customResolver` that is Node's
// own resolution.
import path from 'node:path';
import { createRequire } from 'node:module';
import { defineConfig } from 'vitest/config';

const require = createRequire(import.meta.url);

export default defineConfig({
  resolve: {
    alias: [
      {
        find: '@app-config',
        replacement: path.resolve(__dirname, './src/app.config.ts'),
      },
    ],
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./test/setup.ts'],
    alias: [
      {
        find: '@acme/client',
        replacement: require.resolve('@acme/client', {
          paths: [path.resolve(__dirname, './vendor')],
        }),
      },
      {
        find: /^@acme\/content\/(.+)$/,
        replacement: '@acme/content/$1',
        customResolver: (specifier: string): string => {
          return require.resolve(specifier);
        },
      },
      {
        find: /^@\/app\/\(site\)(\/.*)?$/,
        replacement: path.resolve(__dirname, './src/app/[variant]/(site)$1'),
      },
      {
        find: '@',
        replacement: path.resolve(__dirname, './src'),
      },
      {
        find: 'next/font/google',
        replacement: path.resolve(__dirname, './test/mocks/font.ts'),
      },
    ],
  },
});
