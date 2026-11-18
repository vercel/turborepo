import { expect, test } from '@playwright/test';

const slug = 'hello-world';

test('a post renders', async ({ page }) => {
  await page.goto(`/blog/${slug}`);
  await expect(page.getByRole('article')).toContainText(slug);
});
