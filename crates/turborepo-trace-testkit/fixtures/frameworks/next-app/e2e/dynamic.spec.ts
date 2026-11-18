import { test } from '@playwright/test';

async function visit(page, url: string) {
  await page.goto(url);
}

test('dynamic', async ({ page }) => {
  await visit(page, process.env.TARGET_URL!);
});
