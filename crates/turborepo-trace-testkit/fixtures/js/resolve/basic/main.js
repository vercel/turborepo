// CommonJS entry: `require` resolves with the `require` condition.
const rel = require('./src/rel'); // extension probing -> src/rel.js
const dir = require('./src/dir'); // directory index -> src/dir/index.js
const data = require('./data.json'); // JSON leaf
const internal = require('#internal/util'); // package `imports`
const self = require('app/feature'); // package self-reference
const fs = require('fs'); // builtin
const fsp = require('node:fs/promises'); // node: builtin
const main = require('pkg-main'); // `main` field
const cjs = require('pkg-exports'); // `exports` with `require` condition (+ legacy main)
const feature = require('pkg-exports/features/a'); // `exports` subpath pattern
const only = require('pkg-exports-only'); // `exports` without `main`
const scoped = require('@scope/pkg'); // scoped package
const missing = require('does-not-exist'); // unresolved package
const missingRel = require('./nope'); // unresolved relative
const casing = require('./src/REL.js'); // case mismatch
