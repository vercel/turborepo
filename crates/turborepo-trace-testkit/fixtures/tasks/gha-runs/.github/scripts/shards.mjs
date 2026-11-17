import { readdirSync } from 'node:fs';
import { weights } from './weights.mjs';

const packages = readdirSync('packages');
const shards = packages.map((p) => ({ filters: `--filter=./packages/${p}`, weight: weights[p] ?? 1 }));
console.log(`shards=${JSON.stringify(shards)}`);
