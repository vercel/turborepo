import adapter from '@sveltejs/adapter-auto';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	// See https://svelte.dev/docs/kit/integrations#Preprocessors for more information about preprocessors.
	preprocess: vitePreprocess(),
	kit: {
		// adapter-auto supports a limited set of environments. See https://svelte.dev/docs/kit/adapter-auto.
		// If your environment is not supported or you have chosen a specific environment, switch adapters.
		// See https://svelte.dev/docs/kit/adapters for more information.
		adapter: adapter()
	}
};

export default config;
