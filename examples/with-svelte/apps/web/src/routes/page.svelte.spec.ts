import { describe, expect, it } from 'vitest';
import Page from './+page.svelte';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';

describe('+page.svelte', () => {
	it('renders the page title', async () => {
		await render(Page);

		await expect.element(page.getByText('Web')).toBeInTheDocument();
	});

	it('renders CounterView from @repo/ui', async () => {
		await render(Page);

		await expect.element(page.getByText('Current count: 0')).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: '+' })).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: '-' })).toBeInTheDocument();
	});
});
