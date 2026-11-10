const path = require('path');
const fs = require('fs');
const { Worker } = require('worker_threads');
const { fork } = require('child_process');
const helper = require('./lib/helper');

// Pattern requires (context modules)
const messages = require('./locales/' + lang + '.json');
const plugin = require('./plugins/' + name);
const nested = require('./plugins/' + name + '/index');
const none = require('./nothing/' + name + '.js');

// Alternatives
const impl = require(fast ? './impl/fast' : './impl/slow');

// fs reads relative to __dirname
const config = fs.readFileSync(path.join(__dirname, 'data/config.json'));
const datas = fs.readFileSync(path.join(__dirname, 'data', kind + '.json'));
const all = fs.readdirSync(path.join(__dirname, 'templates'));
const exists = fs.existsSync(path.join(__dirname, 'data/optional.json'));
const own = fs.readFileSync(path.join(__dirname, name));

// cwd-relative
const settings = fs.readFileSync('settings.json');
const cwdGlob = fs.readFileSync('conf/' + env + '.yml');

// workers and child processes
new Worker(path.join(__dirname, 'worker.js'));
fork(path.join(__dirname, 'child'));

// assets
const logo = new URL('./assets/logo.png', import.meta.url);
