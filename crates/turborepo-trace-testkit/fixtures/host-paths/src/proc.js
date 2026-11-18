// Process helpers: the parent of a process and the process table, from /proc.
const { readdirSync, readFileSync } = require('node:fs');
function parentOf(pid) {
  return readFileSync(`/proc/${pid}/stat`, 'utf8').split(' ')[3];
}
function pids() {
  return readdirSync('/proc').filter((name) => /^\d+$/.test(name));
}
module.exports = { parentOf, pids };
