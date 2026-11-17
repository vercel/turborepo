import { expect, test } from '@playwright/test';

test('the home page has the headline', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Ship it' })).toBeVisible();
});
