import adapter from '@sveltejs/adapter-auto';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { mdsvex } from 'mdsvex';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [
		sveltekit({
			// See https://svelte.dev/docs/kit/integrations#Preprocessors for more information about preprocessors.
			extensions: ['.svelte', '.svx'],
			preprocess: [vitePreprocess(), mdsvex()],
			// adapter-auto supports a limited set of environments. See https://svelte.dev/docs/kit/adapter-auto.
			// If your environment is not supported or you have chosen a specific environment, switch adapters.
			// See https://svelte.dev/docs/kit/adapters for more information.
			adapter: adapter()
		})
	],
	server: {
		port: 5174
	},
	preview: {
		port: 4174
	}
});
