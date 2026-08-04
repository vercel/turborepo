// Injected via NODE_OPTIONS="--require /path/audit-node.cjs".
// Replaces process.env with a Proxy that records which keys are read.
// Buffers records and flushes on exit; guards against re-entrancy
// (fs calls themselves read process.env internally).
'use strict';
const fs = require('fs');
const out = process.env.ENV_AUDIT_NODE_OUT;
if (out) {
  const realEnv = process.env;
  const buf = [];
  let recording = true;
  const record = (op, key) => {
    if (!recording || typeof key !== 'string') return;
    recording = false;
    buf.push(`${process.pid}\t${op}\t${key}\n`);
    recording = true;
  };
  const flush = () => {
    recording = false;
    try {
      fs.appendFileSync(out, buf.join(''));
    } catch {}
    buf.length = 0;
  };
  process.on('exit', flush);
  process.env = new Proxy(realEnv, {
    get(target, key, recv) {
      record('get', key);
      return Reflect.get(target, key, recv);
    },
    has(target, key) {
      record('has', key);
      return Reflect.has(target, key);
    },
    ownKeys(target) {
      record('ownKeys', '*ALL_KEYS_ENUMERATED*');
      return Reflect.ownKeys(target);
    },
    getOwnPropertyDescriptor(target, key) {
      record('desc', key);
      return Reflect.getOwnPropertyDescriptor(target, key);
    },
    // Writes must go straight to the target: default Proxy set semantics
    // route through defineProperty with the proxy as receiver, which Node's
    // exotic process.env object rejects ("only accepts a configurable,
    // writable, and enumerable data descriptor"), killing any process that
    // sets an env var (npm sets dozens of npm_config_*).
    set(target, key, value) {
      target[key] = value;
      return true;
    },
    deleteProperty(target, key) {
      delete target[key];
      return true;
    },
    defineProperty(target, key, desc) {
      if ('value' in desc) target[key] = desc.value;
      return true;
    },
  });
}
