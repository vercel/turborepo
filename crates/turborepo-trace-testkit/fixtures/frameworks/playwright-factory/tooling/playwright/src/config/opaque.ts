import { defineConfig as playwrightDefineConfig } from '@playwright/test';
import type { PlaywrightTestConfig } from '@playwright/test';
import { specPattern } from '@some/env';

export function defineConfig(cwd: string): PlaywrightTestConfig {
  return playwrightDefineConfig({
    testDir: './',
    testMatch: specPattern(cwd),
  });
}
