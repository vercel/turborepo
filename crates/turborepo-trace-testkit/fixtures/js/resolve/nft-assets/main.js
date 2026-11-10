// Path values emitted as assets although no tracked fs call reads them (nft's rule).
const path = require('path');
const { spawn } = require('child_process');

let configPath = path.join(__dirname, 'data', 'config.json'); // assigned
function template() {
  return __dirname + '/views/page.html'; // returned
}
spawn('convert', ['--in', path.join(__dirname, 'img/logo.png')]); // inside an array argument
fs.readFileSync(`${__dirname}/notes.txt`); // undeclared global `fs`, template literal
const dict = webpackNs().join(process.cwd(), 'assets', 'dict.json'); // bundler namespace + cwd
module.exports.schemas = path.join(__dirname, 'schemas'); // a directory: every file below it
const wasm = path.join(require.resolve('wasm-pkg'), '../engine.wasm'); // next to a package file
path.join(__dirname, 'missing.txt'); // missing: a quiet probe
path.join(__dirname, '..'); // an ancestor: never emitted
console.log(__dirname, __filename, configPath, template, dict, wasm); // the anchors themselves: never
