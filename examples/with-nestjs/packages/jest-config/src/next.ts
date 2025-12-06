// unfortunately, need to disambiguate the `Config` namespace @jest/types uses (via next/jest) and the `Config` type we want for typing our config here
// biome-ignore lint/correctness/noUnusedImports: required for Jest type resolution
import type { Config as ConfigNamespace } from '@jest/types';
import type { Config } from 'jest';
import nextJest from 'next/jest';
import { config as baseConfig } from './base';

const createJestConfig = nextJest({
	dir: './',
});

const config = {
	...baseConfig,
	moduleFileExtensions: [...baseConfig.moduleFileExtensions, 'jsx', 'tsx'],
} as const satisfies Config;

const nextConfig = createJestConfig(config);

export default nextConfig;
