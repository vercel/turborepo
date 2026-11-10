#!/usr/bin/env node
const { runTestRunnerWithDDTrace } = require('../utils');

runTestRunnerWithDDTrace({
  runner: 'vitest',
  defaultSessionName: 'unit-tests-vitest',
});
