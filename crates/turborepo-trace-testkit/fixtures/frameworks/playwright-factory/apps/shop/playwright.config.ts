import { defineConfig } from '@acme/playwright/config';

const config = defineConfig(__dirname);

export default {
  ...config,
  testDir: './spec',
  testMatch: '*.spec.ts',
};
