// A helper that probes candidate shells and skips the missing ones.
const { existsSync } = require('node:fs');
const candidateShells = ['/bin/sh', '/bin/bash', '/bin/dash'];
const found = [];
for (const shell of candidateShells) {
  const skip = !existsSync(shell);
  if (!skip) found.push(shell);
}
module.exports = { found };
