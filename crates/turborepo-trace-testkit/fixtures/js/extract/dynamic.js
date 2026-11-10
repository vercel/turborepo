'use strict';
// Partial evaluation of module specifiers (M2).
const path = require('path');
const { join } = require('node:path');

// Concatenation, templates, String.raw.
require('u' + 'rl');
require(`./tpl/${'fixed'}.js`);
require(String.raw`./raw/file.js`);
require('./plugins/' + name);
require(`./locales/${lang}/index.json`);
require('./a/' + kind + '/b/' + sub + '.js');

// Alternatives.
require(isProd ? './prod' : './dev');
require(a ? './one' : b ? './two' : './three');
require(override || './fallback');
import(mode ?? './lazy-default');

// Constants.
const BASE = './lib';
const SUFFIX = '.node';
const ROOT = path.join(__dirname, '..', 'shared');
require(BASE + '/x');
require(`${BASE}/y${SUFFIX}`);
require(path.join(ROOT, 'z.js'));

// File anchors and path functions.
require(__dirname + '/sibling.js');
require(path.resolve(__dirname, 'deep', '../up.js'));
require(join(__dirname, 'j', name + '.js'));
require(path.dirname(__filename) + '/d.js');
require.resolve(path.join(__dirname, 'r.json'));

// Environment and unknowns.
require(process.env.PLUGIN_PATH);
require('./env/' + process.env.NODE_ENV + '.js');
require(name + '.js');
require(getPath());
require(path.join(cwd, 'x'));
