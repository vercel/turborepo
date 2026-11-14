#!/usr/bin/env node
import { execa } from 'execa';

await execa('tsc', ['-p', 'tsconfig.build.json', ...process.argv.slice(2)], { stdio: 'inherit' });
