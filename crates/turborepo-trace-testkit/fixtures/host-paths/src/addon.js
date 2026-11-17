// A precompiler that loads a native addon installed on the machine image.
const path = require('node:path');
const nodeMajor = process.versions.node.split('.')[0];
function load(options) {
  const addonPath = options.addonPath ?? `/usr/local/lib/bytecode-precompiler-node${nodeMajor}.node`;
  return require(path.resolve(addonPath));
}
module.exports = { load, plugin: () => require('/opt/tool/plugin.js') };
