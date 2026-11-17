// Loader aliases, createRequire, shadowed require, eval (M2).
const { createRequire } = require('module');
const req = createRequire(__filename);
req('./via-create-require');

const r = require;
r('./via-alias');

// A browserify-style bundle: `require` is a parameter of each module wrapper.
(function (modules) {
  // ...
})({
  1: [function (require, module, exports) {
    require('./bundled-a');
    require('./bundled-' + name);
  }, {}],
});
require('./after-bundle');

function local() {
  var require = makeRequire();
  require('./shadowed');
}

eval('require("./in-eval")');
const fn = new Function('return require("./in-function")');
