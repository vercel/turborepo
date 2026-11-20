import { expect, test } from '@playwright/test';

test('the stats endpoint answers', async ({ request }) => {
  const response = await request.get('/api/stats');
  expect(await response.json()).toHaveProperty('visitors');
});
