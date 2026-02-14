import type { Counter } from './types';

/**
 * An example store based on the svelte tutorial for custom stores:
 * https://svelte.dev/blog/runes#Beyond-components
 */
export function newCounter(): Counter {
  let _count = $state(0);
  return {
    get count() {
      return _count;
    },
    decrement() {
      _count -= 1;
    },
    increment() {
      _count += 1;
    }
  };
}
