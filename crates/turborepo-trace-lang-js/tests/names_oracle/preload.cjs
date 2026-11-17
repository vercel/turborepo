// Preload of the names oracle (see oracle.mjs): records every path that code of the project
// under test passes to a file-reading API, and writes the list to $NAMES_ORACLE_OUT at exit.
// Reads made by Node itself (the module loader) are not recorded: the first frame outside
// this file must be a file of the project.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const { fileURLToPath } = require('node:url');

const PROJ = process.env.NAMES_ORACLE_PROJ;
const OUT = process.env.NAMES_ORACLE_OUT;
const seen = [];
const writeOut = fs.writeFileSync.bind(fs);

function fromProject() {
  const prev = Error.stackTraceLimit;
  Error.stackTraceLimit = 30;
  const stack = new Error().stack.split('\n').slice(1);
  Error.stackTraceLimit = prev;
  for (const f of stack) {
    if (f.includes(__filename)) continue;
    return f.includes(PROJ + path.sep);
  }
  return false;
}

function record(kind, p) {
  if (!fromProject()) return;
  let s;
  if (typeof p === 'string') s = p;
  else if (p instanceof URL && p.protocol === 'file:') s = fileURLToPath(p);
  else if (Buffer.isBuffer(p)) s = p.toString();
  else return;
  seen.push({ kind, path: path.resolve(s) });
}

const FILE_READS = ['readFile', 'createReadStream', 'open', 'exists', 'stat', 'lstat', 'access'];
const DIR_READS = ['readdir', 'opendir'];

function patch(obj, name, kind) {
  const orig = obj[name];
  if (typeof orig !== 'function') return;
  obj[name] = function (p, ...rest) {
    record(kind, p);
    return orig.call(this, p, ...rest);
  };
}

for (const [kind, names] of [
  ['file', FILE_READS],
  ['dir', DIR_READS],
]) {
  for (const n of names) {
    patch(fs, `${n}Sync`, kind);
    patch(fs, n, kind);
    patch(fs.promises, n, kind);
  }
}
require('node:module').syncBuiltinESMExports();

process.on('exit', () => {
  try {
    writeOut(OUT, JSON.stringify(seen));
  } catch {}
});
