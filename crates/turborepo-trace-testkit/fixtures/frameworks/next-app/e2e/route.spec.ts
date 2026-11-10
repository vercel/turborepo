import { test } from '@playwright/test';

test('mocked members', async ({ page }) => {
  await page.route('**/api/teams/*/members', (r) => r.fulfill({ json: [] }));
  await page.goto('http://localhost:3000/acme');
  await page.waitForURL('**/acme/settings');
});
