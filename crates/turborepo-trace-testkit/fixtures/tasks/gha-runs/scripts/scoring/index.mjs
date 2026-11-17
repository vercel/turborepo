import { execSync } from 'node:child_process';
import a from '../../packages/a/src/a.js';
import { threshold } from './threshold.mjs';

execSync('turbo run build --filter=@acme/a', { stdio: 'inherit' });
const score = a.a() * 10;
if (score < threshold) process.exit(1);
