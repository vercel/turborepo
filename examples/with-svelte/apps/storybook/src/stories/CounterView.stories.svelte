<script module lang="ts">
	import { defineMeta } from '@storybook/addon-svelte-csf';
	import { CounterView } from '@repo/ui';

	const { Story } = defineMeta({
		title: 'Example/CounterView',
		component: CounterView,
		tags: ['autodocs'],
		parameters: {
			layout: 'centered'
		}
	});
</script>

<Story name="Default" />
