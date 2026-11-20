#!/usr/bin/env node
// Runs the unit tests with the CI reporter: `jest-ci [--shard=N/M] [args...]`.
const { launch } = require('../lib/launch');

launch({ label: 'unit', tool: 'jest' });
