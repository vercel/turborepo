<script lang="ts">
  let count = $state(0);

  function handleClick() {
    count += 1;
  }
</script>

<button onclick={handleClick}>
  clicks: {count}
</button>
