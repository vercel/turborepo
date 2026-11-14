import { expect, test } from '@playwright/test';

test('shows the team', async ({ page }) => {
  await page.goto('/about');
  await expect(page.getByRole('heading', { name: 'Team' })).toBeVisible();
});
