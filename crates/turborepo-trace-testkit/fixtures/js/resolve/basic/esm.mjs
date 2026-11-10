// ESM entry: `import` resolves with the `import` condition.
import esm from 'pkg-exports';
import mod from 'pkg-module'; // `main` only by default; `module` is not a Node field
import rel from './src/rel.js';
export { feature } from 'app/feature';
const lazy = await import('#internal/util');
