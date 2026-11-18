#!/usr/bin/env node
// Jest through its programmatic API, after the shared preparation.
require('./prepare');
require('jest').run(process.argv.slice(2));
