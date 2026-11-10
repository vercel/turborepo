// Workers, child processes and URL assets (M2).
import { Worker } from 'node:worker_threads';
import * as cp from 'child_process';
import { fork } from 'node:child_process';
import path from 'path';

new Worker(new URL('./worker.js', import.meta.url));
new Worker(path.join(import.meta.dirname, 'pool', `${kind}.mjs`));
cp.fork('./scripts/child.js');
fork(path.resolve(import.meta.dirname, 'child2.js'), ['--flag']);

// URL assets anywhere.
const wasm = new URL('./pkg/module_bg.wasm', import.meta.url);
const font = await fetch(new URL(`./fonts/${weight}.woff2`, import.meta.url));
const remote = new URL('https://example.com/x.js', import.meta.url);
const relative = new URL('./not-an-asset', someBase);
