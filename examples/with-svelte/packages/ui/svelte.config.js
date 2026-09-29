import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	// See https://svelte.dev/docs/kit/integrations#Preprocessors for more information about preprocessors.
	preprocess: vitePreprocess()
};

export default config;
