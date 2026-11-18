import { spawnSync } from 'node:child_process';
import b from '../../packages/b/src/b.js';

// Application code the script happens to import: an input, not CI tooling.
const target = process.env.SMOKE_TARGET ?? (b.b() === 2 ? 'all' : 'none');
const r = spawnSync('pnpm', ['run', '--recursive', `test:${target}`], { stdio: 'inherit' });
process.exit(r.status ?? 1);
