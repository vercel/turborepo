import { expect, test } from '@playwright/test';

test('logs in', async ({ page }) => {
  await page.goto('/login');
  await expect(page.getByLabel('Email')).toBeVisible();
});
