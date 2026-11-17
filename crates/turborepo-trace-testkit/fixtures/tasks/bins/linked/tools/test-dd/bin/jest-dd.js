#!/usr/bin/env node
// Jest with tracing in CI: `jest-dd [--type=unit|integration] [jest arguments...]`.
const { runTestRunnerWithDDTrace } = require('../utils');

runTestRunnerWithDDTrace({
  runner: 'jest',
  defaultSessionName: 'unit-tests-jest',
});
