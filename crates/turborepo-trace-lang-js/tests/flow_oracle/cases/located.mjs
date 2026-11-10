// ESM: module loads whose specifier is a file next to a resolved module. `fo-eve` has its
// `import` entry in dist/ and its `require` entry in cjs/: `import.meta.resolve` and
// `require.resolve` find different directories (see oracle.mjs).
import { createRequire } from 'node:module';
import { pathToFileURL, fileURLToPath } from 'node:url';
import path from 'node:path';
const require = createRequire(import.meta.url);
const root1 = new URL('.', import.meta.resolve('fo-eve'));
await import(new URL('inner.js', root1).href).catch(() => {});
await import(new URL('./deep/leaf.js', import.meta.resolve('fo-eve'))).catch(() => {});
await import(new URL('deep/', root1) + 'leaf.js').catch(() => {});
const root2 = path.dirname(createRequire(import.meta.url).resolve('fo-eve/package.json'));
await import(pathToFileURL(path.join(root2, 'dist/inner.js')).href).catch(() => {});
await import(`${root2}/cjs/inner.js`).catch(() => {});
const root3 = new URL('dist/', pathToFileURL(require.resolve('fo-eve/package.json')));
const load3 = (p) => import(new URL(p, root3).href);
await load3('inner.js').catch(() => {});
await load3(process.env.FLOW_K0 === '1' ? 'deep/leaf.js' : 'index.mjs').catch(() => {});
export const load4 = (p) => import(new URL(p, root3).href);
await load4(process.env.FLOW_K1 === '1' ? 'deep/leaf.js' : 'inner.js').catch(() => {});
const entry = pathToFileURL(createRequire(import.meta.url).resolve('fo-eve')).href;
await import(new URL('./inner.js', entry).href).catch(() => {});
await import(pathToFileURL(require.resolve('fo-eve'))).catch(() => {});
await import(fileURLToPath(new URL('../dist/inner.js', entry))).catch(() => {});
await import(new URL('../dist/inner.js?v=1#frag', entry).href).catch(() => {});
const dep = require.resolve('fo-dep', { paths: [fileURLToPath(new URL('.', import.meta.resolve('fo-eve')))] });
await import(pathToFileURL(dep).href).catch(() => {});
const other = createRequire(new URL('../../other/package.json', import.meta.url));
other('fo-dep');
other('./x.js');
await import(pathToFileURL(other.resolve('fo-dep')).href).catch(() => {});
const metaDir = path.dirname(fileURLToPath(import.meta.resolve('fo-eve')));
await import(path.join(metaDir, 'deep', 'leaf.js')).catch(() => {});
