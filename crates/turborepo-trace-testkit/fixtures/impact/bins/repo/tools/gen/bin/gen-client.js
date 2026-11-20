#!/usr/bin/env node
// Reads a schema on stdin and prints a client module generated from it.
const fs = require('fs');
const path = require('path');
const { render } = require('../lib/render');

const header = fs.readFileSync(path.join(__dirname, '../templates/header.txt'), 'utf8');
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => (input += chunk));
process.stdin.on('end', () => process.stdout.write(render(header, JSON.parse(input))));
