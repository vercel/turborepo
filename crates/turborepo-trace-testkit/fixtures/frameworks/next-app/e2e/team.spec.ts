import { test } from '@playwright/test';
import { TeamPage } from './pages/team-page';

test('team settings', async ({ page }) => {
  const slug = `team-${Date.now()}`;
  const tp = new TeamPage(page, slug);
  await tp.goto();
  await tp.openPhoto();
});
