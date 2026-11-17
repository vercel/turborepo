'use strict';
// Shapes seen while tracing a real API server (M2).

// dd-trace: a ternary between two exporters.
const inAWSLambda = process.env.AWS_LAMBDA_FUNCTION_NAME !== undefined;
const Exporter = require(inAWSLambda && !usingLambdaExtension ? './exporters/log' : './exporters/agent');

// msgpackr: a builtin hidden from naive scanners.
const { URL } = require('u' + 'rl');

// datadog wasm rewriter: String.raw specifiers.
const logger = require(String.raw`./snippets/x/tracer_logger.js`);

// A loader helper: the specifier is a parameter.
function load(path) {
  return require(path);
}

module.exports = { Exporter, URL, logger, load };
