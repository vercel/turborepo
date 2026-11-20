import { configDefaults, defineConfig } from 'vite-plus';

export default defineConfig({
  test: {
    exclude: [...configDefaults.exclude, 'src/**/*.slow.test.ts'],
  },
});
