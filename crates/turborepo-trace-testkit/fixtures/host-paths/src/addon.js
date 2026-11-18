// Loads a native module that the machine image provides.
const path = require('node:path');
const abi = process.versions.modules;
function open(config) {
  const file = config.nativeFile ?? `/usr/local/lib/imagecodec-${abi}.node`;
  return require(path.resolve(file));
}
module.exports = { open, extra: () => require('/opt/tool/plugin.js') };
