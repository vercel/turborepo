import type { Page } from '@playwright/test';

export class DashboardPage {
  constructor(private readonly page: Page) {}

  async goto() {
    await this.page.goto('/dashboard');
  }

  async gotoSettings() {
    await this.page.goto('/dashboard/settings');
  }
}
