// An app's vitest config: its `#/…` imports (as in tsconfig.json) map into the project,
// except build outputs, which must stay unresolved. The exception is a negative lookahead
// that `new RegExp` builds from a list of files.
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

const base = fileURLToPath(new URL('.', import.meta.url));

// Written by the build; a clean checkout has none of them.
const BUILT = ['src/manifest.mjs', 'gen/known-ids.mjs', 'gen/build-info.mjs'];

const escaped = BUILT.map((file) => file.replace(/\./g, '\\.')).join('|');

const alias = [
  {
    find: new RegExp(`^#/(?!(?:${escaped})$)(.*)$`),
    replacement: `${base}$1`,
  },
  // Marker modules that only the framework's compiler understands.
  {
    find: 'server-only',
    replacement: fileURLToPath(new URL('./test/stubs/server-only.ts', import.meta.url)),
  },
  {
    find: /^next\/font\/(google|local)$/,
    replacement: fileURLToPath(new URL('./test/stubs/fonts.ts', import.meta.url)),
  },
];

export default defineConfig({
  test: {
    server: { deps: { inline: ['@acme/icons', '@acme/ui'] } },
    setupFiles: ['./test/setup-dom.ts'],
  },
  resolve: { alias },
});
