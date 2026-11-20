import { DashboardPage } from './pages/dashboard-page';
import { expect, test } from '@playwright/test';

test('the dashboard shows visitors', async ({ page }) => {
  const dashboard = new DashboardPage(page);
  await dashboard.goto();
  await expect(page.getByTestId('visitors')).toHaveText('1204');
});

test('settings can be saved', async ({ page }) => {
  const dashboard = new DashboardPage(page);
  await dashboard.gotoSettings();
  await page.getByRole('textbox').fill('Ada');
  await page.getByRole('button', { name: 'Save' }).click();
});
