import { test, expect } from '@playwright/test';

test('health', async ({ request }) => {
  const res = await request.get('/api/health');
  expect(res.ok()).toBeTruthy();
});
