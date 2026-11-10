#!/usr/bin/env node
// Prints a client generated from the schema file named on the command line.
const fs = require('fs');
const path = require('path');
const { render } = require('../lib/render');

const header = fs.readFileSync(path.join(__dirname, '../templates/header.txt'), 'utf8');
const schema = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
process.stdout.write(render(header, schema));
