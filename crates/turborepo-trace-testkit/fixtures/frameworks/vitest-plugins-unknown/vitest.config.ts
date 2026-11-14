// A known plugin next to a loader plugin of another package: what the loader does to
// imports is not known.
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import shaderLoader from '@acme/shaders/loader-vite';

export default defineConfig({
  plugins: [react(), shaderLoader({ minify: true })],
  test: {
    environment: 'jsdom',
  },
});
