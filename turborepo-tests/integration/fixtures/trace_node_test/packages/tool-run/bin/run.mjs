#!/usr/bin/env node
import { spawnSync } from 'node:child_process';

const [program, ...rest] = process.argv.slice(2);
const result = spawnSync(program, rest, { stdio: 'inherit' });
process.exit(result.status ?? 1);
