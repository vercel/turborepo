#!/usr/bin/env node
// Programmatic jest with an extra reporter.
require('./setup');
require('jest').run(process.argv.slice(2));
