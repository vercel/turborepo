import { defineConfig as definePlaywright } from '@playwright/test';
import type { PlaywrightTestConfig } from '@playwright/test';
import { specPattern } from '@some/env';

export function defineConfig(dir: string): PlaywrightTestConfig {
  return definePlaywright({ testDir: './', testMatch: specPattern(dir) });
}
