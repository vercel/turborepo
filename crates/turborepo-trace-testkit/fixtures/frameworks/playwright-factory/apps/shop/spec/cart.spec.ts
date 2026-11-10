import { expect, test } from '@playwright/test';

test('adds to the cart', async ({ page }) => {
  await page.goto('/products/1');
  await page.getByRole('button', { name: 'Add' }).click();
  await expect(page.getByTestId('cart-count')).toHaveText('1');
});
