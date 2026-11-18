// Lists the shells this machine has, in order of preference.
const { existsSync } = require('node:fs');
const preferred = ['/bin/sh', '/bin/bash', '/bin/dash'];
const available = [];
for (const candidate of preferred) {
  if (existsSync(candidate)) {
    available.push(candidate);
  }
}
module.exports = { available };
