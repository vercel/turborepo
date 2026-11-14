import { join } from 'node:path';
import { devices, defineConfig as playwrightDefineConfig } from '@playwright/test';
import type { PlaywrightTestConfig } from '@playwright/test';
import { loadEnv } from '@some/env';
import { getBaseUrl } from '../utils/get-base-url';

type TestType = 'e2e' | 'testserver';

const DEVTOOLS = process.env.DEVTOOLS;
const TEST_TYPE = (process.env.TEST_TYPE ?? 'e2e') as TestType;
const IS_CI = process.env.CI === 'true';

const testTypeToTestMatch: Record<TestType, string> = {
  e2e: '*.e2e.ts',
  testserver: '*.spec.ts',
};

function testTypeFolder(cwd: string, path: string): string {
  const folder = [cwd, TEST_TYPE];
  return join(...folder, path);
}

export function defineConfig(cwd: string): PlaywrightTestConfig {
  loadEnv(cwd);

  return playwrightDefineConfig({
    globalSetup: join(__dirname, '../../src', 'config/global-setup.ts'),
    globalTeardown: join(__dirname, '../../src', 'config/global-teardown.ts'),
    testDir: './',
    testMatch: testTypeToTestMatch[TEST_TYPE],
    outputDir: testTypeFolder(cwd, 'results/output'),
    timeout: 30_000 * (TEST_TYPE === 'testserver' ? 2 : 1),
    retries: IS_CI ? 2 : 0,
    reporter: [
      ['html', { outputFolder: testTypeFolder(cwd, 'results/report'), open: 'never' }],
    ],
    use: {
      baseURL: getBaseUrl(),
      trace: 'retain-on-failure',
    },
    projects: [
      {
        name: 'chromium',
        use: {
          ...devices['Desktop Chrome'],
          viewport: DEVTOOLS
            ? { width: 1600, height: 1200 }
            : devices['Desktop Chrome'].viewport,
        },
        testMatch: [
          testTypeToTestMatch[TEST_TYPE],
          `*.${TEST_TYPE === 'e2e' ? 'e2e' : 'spec'}.chromium.ts`,
        ],
      },
      {
        name: 'firefox',
        use: { ...devices['Desktop Firefox'] },
      },
      {
        name: 'webkit',
        use: { ...devices['Desktop Safari'] },
      },
    ],
  });
}
