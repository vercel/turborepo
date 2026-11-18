import type { Page } from '@playwright/test';

export class BasePage {
  constructor(readonly page: Page) {}
}

export class TeamPage extends BasePage {
  constructor(page: Page, readonly team: string) {
    super(page);
  }
  path() {
    return `/${this.team}/settings`;
  }
  async goto() {
    await this.page.goto(this.path());
  }
  async openPhoto() {
    await this.page.locator('a[href="/acme/photo/1"]').click();
  }
}
