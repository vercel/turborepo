// Preload of the flow oracle's cases for modules that are not installed (see
// notinstalled.mjs): records every file the process loads, through `require`, `import()`
// or a static import, with symlinks resolved, and writes the list to $NOTINSTALLED_OUT at
// exit (also after a crash). Files inside `node_modules` are recorded too: the cases are
// about whether a load stays inside an installed package.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { fileURLToPath } = require('node:url');

const OUT = process.env.NOTINSTALLED_OUT;
const loaded = new Set();
const errors = [];
const writeOut = fs.writeFileSync.bind(fs);
const realpath = fs.realpathSync.bind(fs);

function note(file) {
  if (typeof file !== 'string' || !path.isAbsolute(file)) return;
  try {
    loaded.add(realpath(file));
  } catch {
    loaded.add(file);
  }
}

if (typeof Module.registerHooks === 'function') {
  Module.registerHooks({
    load(url, context, next) {
      if (url.startsWith('file:')) note(fileURLToPath(url));
      return next(url, context);
    },
  });
}
const origLoad = Module._load;
Module._load = function (request, parent, ...rest) {
  try {
    note(Module._resolveFilename(request, parent));
  } catch {}
  return origLoad.call(this, request, parent, ...rest);
};

function fail(e) {
  errors.push(String(e && e.message ? e.message : e).split('\n')[0]);
}

process.on('exit', () => {
  for (const file of Object.keys(require.cache)) note(file);
  try {
    writeOut(OUT, JSON.stringify({ loaded: [...loaded].sort(), errors }));
  } catch {}
});
process.on('uncaughtException', (e) => {
  fail(e);
  process.exitCode = 0;
});
process.on('unhandledRejection', fail);
