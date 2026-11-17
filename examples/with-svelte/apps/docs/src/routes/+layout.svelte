<script lang="ts">
	const { children } = $props();
</script>

<main>
	{@render children()}
</main>

<style>
	main {
		max-width: 60rem;
		margin: 0 auto;
		padding: 2rem;
	}
</style>
