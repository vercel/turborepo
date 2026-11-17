require('sync-pkg'); // `module-sync` (Node >= 22) and the `require` fallback
require('#internal'); // the same for `imports`
require('browser-pkg'); // with the `browser` condition: the `browser` field remapping too
require('gyp-pkg'); // node-gyp-build prebuilds in libc-tagged directories
