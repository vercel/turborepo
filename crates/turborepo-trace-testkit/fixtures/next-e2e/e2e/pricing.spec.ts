import { expect, test } from '@playwright/test';

test('the calculator multiplies seats', async ({ page }) => {
  await page.goto('/pricing');
  await page.getByLabel('Seats').fill('3');
  await expect(page.getByRole('status')).toHaveText('60');
});

test('the old address still works', async ({ page }) => {
  await page.goto('/plans');
  await expect(page.getByRole('heading', { name: 'Pricing' })).toBeVisible();
});
