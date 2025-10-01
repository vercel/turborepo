import { config } from '@repo/eslint-config/index.js';

export default [
	...config,
	{
		ignores: ['.svelte-kit/*']
	}
];
