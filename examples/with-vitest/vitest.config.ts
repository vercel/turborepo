import { defineConfig } from 'vitest/config';
import { sharedConfig } from '@repo/vitest-config';

export default defineConfig({
  ...sharedConfig,
  projects: [
    {
      name: 'packages',
      root: './packages/*',
      test: {
        ...sharedConfig.test,
        // Project-specific configuration for packages
      }
    },
    {
      name: 'apps',
      root: './apps/*',
      test: {
        ...sharedConfig.test,
        // Project-specific configuration for apps
        environment: 'jsdom',
      }
    }
  ]
});