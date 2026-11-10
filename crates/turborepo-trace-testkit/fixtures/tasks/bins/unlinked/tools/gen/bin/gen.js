#!/usr/bin/env node
const fs = require('fs');
const { render } = require('../lib/render');

process.stdout.write(render(JSON.parse(fs.readFileSync(process.argv[2], 'utf8'))));
