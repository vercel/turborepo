import { test } from '@playwright/test';
import { computeUrl } from './compute';

test.use({ routes: ['/dashboard'] });

test('annotated', async ({ page }) => {
  await page.goto(computeUrl());
});
