<script lang="ts">
  import type { PageData } from "./$types";

  export let data: PageData;
</script>

<h1>Welcome to SvelteKit</h1>
<p>Visit <a href="https://kit.svelte.dev">kit.svelte.dev</a> to read the documentation</p>

<br/>

<h3>Users</h3>

{#if data.users.length === 0}
  <p>No users found</p>
{:else}
  {#each data.users as user}
    <ul>
      <li>{user.name}</li>
    </ul>
  {/each}
{/if}
