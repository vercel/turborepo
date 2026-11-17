import { describe, expect, it } from 'vitest';
import Page from './+page.svx';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';

describe('+page.svx', () => {
	it('renders the page title', async () => {
		await render(Page);

		await expect.element(page.getByText('Docs')).toBeInTheDocument();
	});

	it('renders CounterView from @repo/ui', async () => {
		await render(Page);

		await expect.element(page.getByText(/Current count: 0/)).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: '+' })).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: '-' })).toBeInTheDocument();
	});
});
