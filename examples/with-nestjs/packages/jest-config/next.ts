import type { Config } from 'jest';
// unfortunately, need to disambiguate the `Config` namespace @jest/types uses (via next/jest) and the `Config` type we want for typing our config here
import type { Config as ConfigNamespace } from '@jest/types';
import nextJest from 'next/jest';
import { config as baseConfig } from './base';

const createJestConfig = nextJest({
  dir: './',
});

const config = {
  ...baseConfig,
  moduleFileExtensions: [...baseConfig.moduleFileExtensions, 'jsx', 'tsx'],
} as const satisfies Config;

export default createJestConfig(config);
