// Loaders nft follows beyond `require('literal')`.
const load = (name) => require(name); // a require wrapper
load('./plugins/a');

var resolveFrom = require('resolve-from'); // resolve-from relative to __dirname
var target = resolveFrom(__dirname, './plugins/b.js');
require(target);

var lib = process.env.COV ? './lib-cov' : './lib'; // a top-level `var` with alternatives
require(lib + '/index');

const pino = require('pino'); // pino transports load their target in a worker
pino.transport({ target: 'my-transport' });

const { register } = require('module'); // ESM loader hooks
const { pathToFileURL } = require('url');
register('./hooks/loader.mjs', pathToFileURL(__filename));

require('./bundle.js'); // a browserify bundle
