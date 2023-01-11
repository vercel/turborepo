"use strict";

/**
 * Webpack loader
 *
 * @todo once webpack supports ESM loaders, remove this wrapper.
 *
 * @param {string} code
 */
module.exports = function (code) {
  const callback = this.async();
  // Note that `import()` caches, so this should be fast enough.
  import("./loader.js").then((module) =>
    module.loader.call(this, code, callback)
  );
};
