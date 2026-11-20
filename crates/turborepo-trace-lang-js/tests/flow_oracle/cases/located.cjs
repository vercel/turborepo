// CommonJS: module loads resolved from another place, and files next to a resolved module.
// `fo-dep` exists next to this package, next to `other/` and inside `fo-eve`: a search from
// the wrong place finds another copy (see oracle.mjs).
const { createRequire } = require('node:module');
const { pathToFileURL } = require('node:url');
const path = require('node:path');
const found1 = require.resolve('fo-dep', { paths: [require.resolve('fo-eve/package.json')] });
require(found1);
const found2 = require.resolve('fo-dep/package.json', { paths: [path.join(__dirname, '../../other')] });
require(found2);
const found3 = require.resolve('./x.js', { paths: [path.join(__dirname, '../../other')] });
require(found3);
const found4 = require.resolve('fo-dep', { paths: [__dirname, path.join(__dirname, '../../other')] });
require(found4);
const found5 = require.resolve('fo-dep', { paths: [path.join(__dirname, 'nowhere'), path.dirname(require.resolve('fo-eve'))] });
require(found5);
const options = { paths: [path.join(__dirname, '../../other')] };
const found6 = require.resolve('fo-dep', options);
require(found6);
const other = createRequire(path.join(__dirname, '../../other/package.json'));
other('./x.js');
other('fo-dep');
other.resolve('fo-dep/package.json');
other.resolve('./x.js');
createRequire(path.join(__dirname, '..', 'package.json'))('./src/fx/own.cjs');
createRequire(__filename).resolve('fo-dep');
createRequire(require.resolve('fo-eve/package.json'))('fo-dep');
createRequire(require.resolve('fo-eve/package.json'))('./cjs/inner.js');
createRequire(__dirname + '/')('./fx/own.cjs');
createRequire(path.join(__dirname, 'fx/'))('./own.cjs');
createRequire(path.join(__dirname, 'fx') + '/')('./own.cjs');
createRequire(__dirname)('./package.json');
const own = createRequire(__filename);
own('fo-dep');
function scoped() { const require = createRequire(path.join(__dirname, '../../other/noop.js')); return require('./x.js'); }
scoped();
function scopedBare() { const require = createRequire(path.join(__dirname, '../../other/noop.js')); return require('fo-dep'); }
scopedBare();
let either = createRequire(__filename);
if (process.env.FLOW_K0 === '1') either = createRequire(path.join(__dirname, '../../other/a.js'));
either('fo-dep');
const viaUrl = createRequire(pathToFileURL(path.join(__dirname, '../../other/b.js')));
viaUrl('fo-dep');
const viaHref = createRequire(pathToFileURL(path.join(__dirname, '../../other/b.js')).href);
viaHref('./x.js');
const anchors = { other: path.join(__dirname, '../../other/c.js') };
createRequire(anchors.other)('fo-dep');
let late = createRequire(path.join(__dirname, '../../other/e.js'));
function useLate() { return late('fo-dep'); }
late = createRequire(__filename);
useLate();
const nested = createRequire(createRequire(require.resolve('fo-eve/package.json')).resolve('fo-dep'));
nested('./package.json');
const evePkg = require.resolve('fo-eve/package.json');
require(path.join(evePkg, '../cjs/inner.js'));
require(path.join(path.dirname(require.resolve('fo-eve')), 'inner'));
require(path.dirname(evePkg) + '/dist/inner.js');
require(`${path.dirname(evePkg)}/dist/deep/leaf.js`);
import(pathToFileURL(require.resolve('fo-eve'))).catch(() => {});
import(pathToFileURL(path.join(evePkg, '..', 'dist', 'inner.js')).href).catch(() => {});
const dir = process.env.FLOW_PRESENT;
try { createRequire(path.join(__dirname, dir, 'x.js'))('fo-dep'); } catch {}
try { require(require.resolve('fo-dep', { paths: [path.join(__dirname, dir)] })); } catch {}
// An absolute path and a builtin module are the same for every `require` function.
const somewhere = createRequire(path.join(__dirname, dir, 'y.js'));
somewhere(path.join(__dirname, 'fx/own.cjs'));
somewhere.resolve(path.join(__dirname, '../../other/x.js'));
somewhere('node:path');
other(path.join(__dirname, 'fx/own.cjs'));
other('fs');
function builtins() { const require = createRequire(path.join(__dirname, dir, 'z.js')); const fs = require('fs'); return fs.readFileSync(path.join(__dirname, 'fx/a.json')); }
builtins();
