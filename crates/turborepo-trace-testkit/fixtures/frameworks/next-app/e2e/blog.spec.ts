import { test } from '@playwright/test';
import { blogPath, gotoPricing } from './helpers';

test('blog', async ({ page }) => {
  await page.goto(blogPath('hello'));
  await page.goto('/blog/new?draft=1');
  await page.goto('/old-blog/first');
  await gotoPricing(page);
});
