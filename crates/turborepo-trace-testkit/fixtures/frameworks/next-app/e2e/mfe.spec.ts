import { test } from '@playwright/test';

test('elsewhere', async ({ page }) => {
  await page.goto('/guides/intro');
  await page.goto('/docs/getting-started');
  await page.goto('/help');
  await page.goto('/favicon.ico');
});
