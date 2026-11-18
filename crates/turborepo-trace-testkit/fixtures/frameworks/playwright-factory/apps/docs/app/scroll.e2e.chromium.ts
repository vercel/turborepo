import { expect, test } from '@playwright/test';

test('no layout shift on the home page', async ({ page }) => {
  await page.goto('/');
  const shift = await page.evaluate(() => (window as any).__cls ?? 0);
  expect(shift).toBeLessThan(0.1);
});
