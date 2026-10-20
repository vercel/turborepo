import { describe, expect, it } from 'vitest';
import { newCounter } from './counter.svelte';

describe('counter store', () => {
	const counter = newCounter();

	it('starts with 0', () => {
		expect(counter.count).toBe(0);
	});
});
