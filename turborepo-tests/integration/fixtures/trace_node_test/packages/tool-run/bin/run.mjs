#!/usr/bin/env node
import { spawnSync } from 'node:child_process';

import { environment } from '../src/environment.mjs';

const [program, ...rest] = process.argv.slice(2);
const result = spawnSync(program, rest, { stdio: 'inherit', env: environment() });
process.exit(result.status ?? 1);
