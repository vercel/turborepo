#!/usr/bin/env node
const { launch } = require('../lib/launch');

launch({ label: 'unit', tool: 'vitest' });
