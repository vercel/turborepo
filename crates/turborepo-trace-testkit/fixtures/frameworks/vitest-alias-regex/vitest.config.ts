// A docs app's vitest config: the `#/…` aliases of tsconfig.json, mirrored for vitest, except
// for build outputs that must stay unresolved. The exception is a negative lookahead built
// from a list of files with `new RegExp`.
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

const projectRoot = fileURLToPath(new URL('.', import.meta.url));

// Generated files, absent from a clean checkout.
const GENERATED = [
  'app/routes.mjs',
  'lib/valid-paths.mjs',
  'lib/flag-codes.mjs',
];

const generatedPattern = GENERATED.map((path) =>
  path.replace(/\./g, '\\.'),
).join('|');

const alias = [
  {
    find: new RegExp(`^#/(?!(?:${generatedPattern})$)(.*)$`),
    replacement: `${projectRoot}$1`,
  },
  // A build-time marker with no runtime module.
  {
    find: 'server-only',
    replacement: fileURLToPath(
      new URL('./test/server-only-stub.ts', import.meta.url),
    ),
  },
  // Compiler-only modules.
  {
    find: /^next\/font\/(?:local|google)$/,
    replacement: fileURLToPath(new URL('./test/font-stub.ts', import.meta.url)),
  },
];

export default defineConfig({
  resolve: { alias },
  test: {
    setupFiles: ['./vitest-setup.ts'],
    server: {
      deps: {
        inline: ['geist', '@acme/ui'],
      },
    },
  },
});
