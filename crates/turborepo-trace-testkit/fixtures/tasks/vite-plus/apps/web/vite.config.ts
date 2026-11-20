import { defineConfig } from 'vite-plus';

export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    setupFiles: ['./test/setup.ts'],
  },
});
