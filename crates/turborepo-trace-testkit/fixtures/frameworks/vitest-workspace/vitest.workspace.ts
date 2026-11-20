import { defineWorkspace } from 'vitest/config';

export default defineWorkspace([
  'packages/*',
  '!packages/legacy',
  {
    extends: './vitest.shared.ts',
    test: {
      name: 'scripts',
      include: ['scripts/**/*.test.ts'],
      environment: 'node',
    },
  },
]);
