import { defineConfig } from '@acme/browser-kit/preset';

const base = defineConfig(__dirname);

export default {
  ...base,
  testDir: './spec',
  testMatch: '*.spec.ts',
};
