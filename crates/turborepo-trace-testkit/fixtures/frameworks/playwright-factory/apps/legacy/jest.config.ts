import { defineProject } from '@acme/playwright/project';

export default defineProject({
  testMatch: ['<rootDir>/src/**/*.test.ts'],
});
