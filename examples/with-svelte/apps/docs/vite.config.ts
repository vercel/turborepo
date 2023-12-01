import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit()],
  test: {
    include: ['src/**/*.{test,spec}.{js,ts}']
  },
  optimizeDeps: {
    include: ['@repo/ui'],
  },
  build: {
    commonjsOptions: {
      include: [/@repo-ui/, /node_modules/],
    },
  },
});
