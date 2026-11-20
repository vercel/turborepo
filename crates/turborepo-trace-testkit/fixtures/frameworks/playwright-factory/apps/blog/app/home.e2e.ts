import { expect, test } from '@playwright/test';

test('lists the posts', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('article')).not.toHaveCount(0);
});
