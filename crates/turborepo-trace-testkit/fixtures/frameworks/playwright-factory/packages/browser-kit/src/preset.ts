import { join } from 'node:path';
import { defineConfig as definePlaywright, devices } from '@playwright/test';
import type { PlaywrightTestConfig } from '@playwright/test';
import { loadEnv } from '@some/env';
import { origin } from './origin';

type Suite = 'browser' | 'api';

// Which suite runs is up to the environment: either pattern is possible.
const suite = (process.env.E2E_SUITE ?? 'browser') as Suite;
const wide = process.env.WIDE_VIEWPORT;
const onCi = process.env.CI === '1';

const patterns: Record<Suite, string> = {
  browser: '*.e2e.ts',
  api: '*.spec.ts',
};

function artifacts(dir: string, name: string): string {
  return join(dir, '.e2e', suite, name);
}

export function defineConfig(dir: string): PlaywrightTestConfig {
  loadEnv(dir);

  return definePlaywright({
    testDir: './',
    testMatch: patterns[suite],
    globalSetup: join(__dirname, '..', 'src/hooks/start.ts'),
    globalTeardown: join(__dirname, '..', 'src/hooks/stop.ts'),
    outputDir: artifacts(dir, 'out'),
    reporter: [['list'], ['json', { outputFile: artifacts(dir, 'results.json') }]],
    workers: onCi ? 4 : undefined,
    use: { baseURL: origin(), screenshot: 'only-on-failure' },
    projects: [
      {
        name: 'chromium',
        testMatch: [patterns[suite], `*.${suite === 'browser' ? 'e2e' : 'spec'}.chromium.ts`],
        use: {
          ...devices['Desktop Chrome'],
          viewport: wide ? { width: 1920, height: 1080 } : devices['Desktop Chrome'].viewport,
        },
      },
      { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
      { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    ],
  });
}
