import { expect, test } from '@playwright/test';

test('navigates to the guide', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('link', { name: 'Guide' }).click();
  await expect(page).toHaveURL('/guide');
});
