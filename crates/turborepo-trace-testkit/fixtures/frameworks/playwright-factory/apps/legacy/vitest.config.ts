import { defineProject } from '@acme/playwright/project';

export default defineProject({
  test: { include: ['src/**/*.test.ts'] },
});
