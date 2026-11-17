import { defineConfig, mergeConfig, configDefaults } from 'vitest/config';
import viteConfig from './vite.config';

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: 'jsdom',
      setupFiles: ['./test/setup.ts'],
      globalSetup: './test/global.ts',
      include: ['src/**/*.{test,spec}.ts'],
      exclude: [...configDefaults.exclude, 'e2e/**'],
      alias: { '@test': new URL('./test', import.meta.url).pathname },
    },
  }),
);
