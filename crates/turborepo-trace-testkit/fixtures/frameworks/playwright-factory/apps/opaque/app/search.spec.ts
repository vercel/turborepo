import { expect, test } from '@playwright/test';

test('finds a page', async ({ page }) => {
  await page.goto('/search?q=guide');
  await expect(page.getByRole('listitem')).not.toHaveCount(0);
});
