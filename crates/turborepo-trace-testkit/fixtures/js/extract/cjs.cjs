'use strict';
// CommonJS forms.
const fs = require('fs');
const { join, resolve: resolvePath } = require('node:path');
const local = require('./local');
const parent = require("../parent/index.js");
const tpl = require(`./template`);
const resolved = require.resolve('./resolved');
const viaModule = module.require('./via-module');
const chained = require('./chained').sub.prop;
const paren = require(('./parenthesized'));

try {
  require('optional-dep');
  const nested = function () { return require('optional-nested'); };
} catch (e) {
  require('in-catch-not-optional');
}

// Computed: unknown specifiers with the argument text.
const dyn = require(name);
const partial = require('./plugins/' + plugin);
const templ = require(`./locale/${lang}.json`);

// Not references.
obj.require('./nope');
something.module.require('./nope');
function notRequire(require) { return require; }
const shape = { require(id) { return id; } };
if (typeof require === 'function' && require.cache) {}
require.resolve.paths('./nope');
define(['require', 'dep'], function (require) {});
require(['amd-a', 'amd-b'], function (a, b) {});
const str = "require('./in-string')";
const re = /require\('\.\/in-regex'\)/g;
// require('./in-comment')
/* require('./in-block-comment') */
module.exports = { fs, join, resolvePath, local, parent };
