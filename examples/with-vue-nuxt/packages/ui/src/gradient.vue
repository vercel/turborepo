<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    small?: boolean;
    conic?: boolean;
    class?: string;
  }>(),
  {},
);
</script>

<template>
  <span
    :class="
      [
        'gradient',
        conic ? 'glowConic' : undefined,
        small ? 'gradientSmall' : 'gradientLarge',
        props.class,
      ]
        .filter(Boolean)
        .join(' ')
    "
  />
</template>
