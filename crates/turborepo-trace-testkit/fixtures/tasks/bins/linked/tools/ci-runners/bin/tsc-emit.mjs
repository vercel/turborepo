#!/usr/bin/env node
import { execa } from 'execa';

const extra = process.argv.slice(2);
await execa('tsc', ['--project', 'tsconfig.build.json', ...extra], { stdio: 'inherit' });
