import { test, expect } from '@playwright/test';

test.describe('Root page', () => {
  test('should reach the home page', async ({ page }) => {
    const response = await page.request.get('/');

    await expect(response).toBeOK();
  });
});
